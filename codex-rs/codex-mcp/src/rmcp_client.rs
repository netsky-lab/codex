//! RMCP client lifecycle for MCP server connections.
//!
//! This module owns startup of individual RMCP clients: building the transport,
//! initializing the server, listing raw tools, applying per-server tool filters,
//! and exposing cached startup snapshots while a client is still connecting.
//! Higher-level aggregation and resource/tool APIs live in
//! [`crate::connection_manager`].

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::collections::hash_map::DefaultHasher;
use std::env;
use std::ffi::OsString;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use crate::codex_apps::CachedCodexAppsToolsLoad;
use crate::codex_apps::CodexAppsToolsCacheContext;
use crate::codex_apps::filter_disallowed_codex_apps_tools;
use crate::codex_apps::load_cached_codex_apps_tools;
use crate::codex_apps::load_startup_cached_codex_apps_tools_snapshot;
use crate::codex_apps::normalize_codex_apps_callable_name;
use crate::codex_apps::normalize_codex_apps_callable_namespace;
use crate::codex_apps::normalize_codex_apps_tool_title;
use crate::codex_apps::write_cached_codex_apps_tools_if_needed;
use crate::elicitation::ElicitationRequestManager;
use crate::mcp::CODEX_APPS_MCP_SERVER_NAME;
use crate::mcp::ToolPluginProvenance;
use crate::runtime::McpRuntimeEnvironment;
use crate::runtime::emit_duration;
use crate::tools::ToolFilter;
use crate::tools::ToolInfo;
use crate::tools::filter_tools;
use crate::tools::tool_with_model_visible_input_schema;
use anyhow::Result;
use anyhow::anyhow;
use async_channel::Sender;
use codex_api::SharedAuthProvider;
use codex_async_utils::CancelErr;
use codex_async_utils::OrCancelExt;
use codex_config::McpServerChannelConfig;
use codex_config::McpServerChannelMode;
use codex_config::McpServerConfig;
use codex_config::McpServerTransportConfig;
use codex_config::types::OAuthCredentialsStoreMode;
use codex_exec_server::HttpClient;
use codex_exec_server::ReqwestHttpClient;
use codex_protocol::protocol::ChannelMessageEvent;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_rmcp_client::ExecutorStdioServerLauncher;
use codex_rmcp_client::LocalStdioServerLauncher;
use codex_rmcp_client::RmcpClient;
use codex_rmcp_client::SendCustomNotification;
use codex_rmcp_client::StdioServerLauncher;
use futures::future::BoxFuture;
use futures::future::FutureExt;
use futures::future::Shared;
use rmcp::model::ClientCapabilities;
use rmcp::model::ElicitationCapability;
use rmcp::model::FormElicitationCapability;
use rmcp::model::Implementation;
use rmcp::model::InitializeRequestParams;
use rmcp::model::JsonObject;
use rmcp::model::ProtocolVersion;
use rmcp::model::Tool as RmcpTool;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use tracing::warn;

/// MCP server capability indicating that Codex should include [`SandboxState`]
/// in tool-call request `_meta` under this key.
pub const MCP_SANDBOX_STATE_META_CAPABILITY: &str = "codex/sandbox-state-meta";
pub const MCP_CHANNEL_NOTIFICATIONS_CAPABILITY: &str = "codex/channel-notifications";

pub(crate) const MCP_TOOLS_LIST_DURATION_METRIC: &str = "codex.mcp.tools.list.duration_ms";
pub(crate) const MCP_TOOLS_FETCH_UNCACHED_DURATION_METRIC: &str =
    "codex.mcp.tools.fetch_uncached.duration_ms";
pub(crate) const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
pub(crate) const DEFAULT_TOOL_TIMEOUT: Duration = Duration::from_secs(120);

const UNTRUSTED_CONNECTOR_META_KEYS: &[&str] = &[
    "connector_id",
    "connector_name",
    "connector_display_name",
    "connector_description",
    "connectorDescription",
];

#[derive(Clone)]
pub(crate) struct ManagedClient {
    pub(crate) client: Arc<RmcpClient>,
    pub(crate) tools: Vec<ToolInfo>,
    pub(crate) tool_filter: ToolFilter,
    pub(crate) tool_timeout: Option<Duration>,
    pub(crate) server_instructions: Option<String>,
    pub(crate) server_supports_sandbox_state_meta_capability: bool,
    pub(crate) codex_apps_tools_cache_context: Option<CodexAppsToolsCacheContext>,
}

impl ManagedClient {
    fn listed_tools(&self) -> Vec<ToolInfo> {
        let total_start = Instant::now();
        if let Some(cache_context) = self.codex_apps_tools_cache_context.as_ref()
            && let CachedCodexAppsToolsLoad::Hit(tools) =
                load_cached_codex_apps_tools(cache_context)
        {
            emit_duration(
                MCP_TOOLS_LIST_DURATION_METRIC,
                total_start.elapsed(),
                &[("cache", "hit")],
            );
            return filter_tools(tools, &self.tool_filter);
        }

        if self.codex_apps_tools_cache_context.is_some() {
            emit_duration(
                MCP_TOOLS_LIST_DURATION_METRIC,
                total_start.elapsed(),
                &[("cache", "miss")],
            );
        }

        self.tools.clone()
    }
}

#[derive(Clone)]
pub(crate) struct AsyncManagedClient {
    pub(crate) client: Shared<BoxFuture<'static, Result<ManagedClient, StartupOutcomeError>>>,
    pub(crate) startup_snapshot: Option<Vec<ToolInfo>>,
    pub(crate) startup_complete: Arc<AtomicBool>,
    pub(crate) tool_plugin_provenance: Arc<ToolPluginProvenance>,
    pub(crate) cancel_token: CancellationToken,
}

impl AsyncManagedClient {
    // Keep this constructor flat so the startup inputs remain readable at the
    // single call site instead of introducing a one-off params wrapper.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        server_name: String,
        config: McpServerConfig,
        store_mode: OAuthCredentialsStoreMode,
        cancel_token: CancellationToken,
        tx_event: Sender<Event>,
        elicitation_requests: ElicitationRequestManager,
        codex_apps_tools_cache_context: Option<CodexAppsToolsCacheContext>,
        tool_plugin_provenance: Arc<ToolPluginProvenance>,
        runtime_environment: McpRuntimeEnvironment,
        runtime_auth_provider: Option<SharedAuthProvider>,
    ) -> Self {
        let tool_filter = ToolFilter::from_config(&config);
        let startup_snapshot = load_startup_cached_codex_apps_tools_snapshot(
            &server_name,
            codex_apps_tools_cache_context.as_ref(),
        )
        .map(|tools| filter_tools(tools, &tool_filter));
        let startup_tool_filter = tool_filter;
        let startup_complete = Arc::new(AtomicBool::new(false));
        let startup_complete_for_fut = Arc::clone(&startup_complete);
        let cancel_token_for_fut = cancel_token.clone();
        let fut = async move {
            let outcome = match async {
                if let Err(error) = validate_mcp_server_name(&server_name) {
                    return Err(error.into());
                }

                let client = Arc::new(
                    make_rmcp_client(
                        &server_name,
                        config.clone(),
                        store_mode,
                        runtime_environment,
                        runtime_auth_provider,
                    )
                    .await?,
                );
                start_server_task(
                    server_name,
                    client,
                    StartServerTaskParams {
                        startup_timeout: config
                            .startup_timeout_sec
                            .or(Some(DEFAULT_STARTUP_TIMEOUT)),
                        tool_timeout: config.tool_timeout_sec.unwrap_or(DEFAULT_TOOL_TIMEOUT),
                        tool_filter: startup_tool_filter,
                        channel_config: config.channel.clone(),
                        tx_event,
                        elicitation_requests,
                        codex_apps_tools_cache_context,
                    },
                )
                .await
            }
            .or_cancel(&cancel_token_for_fut)
            .await
            {
                Ok(result) => result,
                Err(CancelErr::Cancelled) => Err(StartupOutcomeError::Cancelled),
            };

            startup_complete_for_fut.store(true, Ordering::Release);
            outcome
        };
        let client = fut.boxed().shared();
        if startup_snapshot.is_some() {
            let startup_task = client.clone();
            tokio::spawn(async move {
                let _ = startup_task.await;
            });
        }

        Self {
            client,
            startup_snapshot,
            startup_complete,
            tool_plugin_provenance,
            cancel_token,
        }
    }

    pub(crate) async fn client(&self) -> Result<ManagedClient, StartupOutcomeError> {
        self.client.clone().await
    }

    pub(crate) async fn shutdown(&self) {
        self.cancel_token.cancel();
        match self.client().await {
            Ok(client) => client.client.shutdown().await,
            Err(StartupOutcomeError::Cancelled) => {}
            Err(error) => {
                warn!("failed to initialize MCP client during shutdown: {error:#}");
            }
        }
    }

    fn startup_snapshot_while_initializing(&self) -> Option<Vec<ToolInfo>> {
        if !self.startup_complete.load(Ordering::Acquire) {
            return self.startup_snapshot.clone();
        }
        None
    }

    pub(crate) async fn listed_tools(&self) -> Option<Vec<ToolInfo>> {
        let annotate_tools = |tools: Vec<ToolInfo>| {
            let mut tools = tools;
            for tool in &mut tools {
                if tool.server_name == CODEX_APPS_MCP_SERVER_NAME {
                    tool.tool = tool_with_model_visible_input_schema(&tool.tool);
                }

                let plugin_names = match tool.connector_id.as_deref() {
                    Some(connector_id) => self
                        .tool_plugin_provenance
                        .plugin_display_names_for_connector_id(connector_id),
                    None => self
                        .tool_plugin_provenance
                        .plugin_display_names_for_mcp_server_name(tool.server_name.as_str()),
                };
                tool.plugin_display_names = plugin_names.to_vec();

                if plugin_names.is_empty() {
                    continue;
                }

                let plugin_source_note = if plugin_names.len() == 1 {
                    format!("This tool is part of plugin `{}`.", plugin_names[0])
                } else {
                    format!(
                        "This tool is part of plugins {}.",
                        plugin_names
                            .iter()
                            .map(|plugin_name| format!("`{plugin_name}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                let description = tool
                    .tool
                    .description
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("");
                let annotated_description = if description.is_empty() {
                    plugin_source_note
                } else if matches!(description.chars().last(), Some('.' | '!' | '?')) {
                    format!("{description} {plugin_source_note}")
                } else {
                    format!("{description}. {plugin_source_note}")
                };
                tool.tool.description = Some(Cow::Owned(annotated_description));
            }
            tools
        };

        // Keep cache payloads raw; plugin provenance is resolved per-session at read time.
        let tools = if let Some(startup_tools) = self.startup_snapshot_while_initializing() {
            Some(startup_tools)
        } else {
            match self.client().await {
                Ok(client) => Some(client.listed_tools()),
                Err(_) => self.startup_snapshot.clone(),
            }
        };
        tools.map(annotate_tools)
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub(crate) enum StartupOutcomeError {
    #[error("MCP startup cancelled")]
    Cancelled,
    // We can't store the original error here because anyhow::Error doesn't implement
    // `Clone`.
    #[error("MCP startup failed: {error}")]
    Failed { error: String },
}

impl From<anyhow::Error> for StartupOutcomeError {
    fn from(error: anyhow::Error) -> Self {
        Self::Failed {
            error: error.to_string(),
        }
    }
}

pub(crate) fn elicitation_capability_for_server(
    _server_name: &str,
) -> Option<ElicitationCapability> {
    // https://modelcontextprotocol.io/specification/2025-06-18/client/elicitation#capabilities
    // indicates this should be an empty object.
    Some(ElicitationCapability {
        form: Some(FormElicitationCapability {
            schema_validation: None,
        }),
        url: None,
    })
}

pub(crate) async fn list_tools_for_client_uncached(
    server_name: &str,
    client: &Arc<RmcpClient>,
    timeout: Option<Duration>,
    server_instructions: Option<&str>,
) -> Result<Vec<ToolInfo>> {
    let resp = client
        .list_tools_with_connector_ids(/*params*/ None, timeout)
        .await?;
    let tools = resp
        .tools
        .into_iter()
        .map(|tool| {
            let mut tool_def = tool.tool;
            let (connector_id, connector_name, connector_description) =
                sanitize_tool_connector_metadata(
                    server_name,
                    &mut tool_def,
                    tool.connector_id,
                    tool.connector_name,
                    tool.connector_description,
                );
            let callable_name = normalize_codex_apps_callable_name(
                server_name,
                &tool_def.name,
                connector_id.as_deref(),
                connector_name.as_deref(),
            );
            let callable_namespace =
                normalize_codex_apps_callable_namespace(server_name, connector_name.as_deref());
            if let Some(title) = tool_def.title.as_deref() {
                let normalized_title =
                    normalize_codex_apps_tool_title(server_name, connector_name.as_deref(), title);
                if tool_def.title.as_deref() != Some(normalized_title.as_str()) {
                    tool_def.title = Some(normalized_title);
                }
            }
            ToolInfo {
                server_name: server_name.to_owned(),
                callable_name,
                callable_namespace,
                server_instructions: server_instructions.map(str::to_string),
                tool: tool_def,
                connector_id,
                connector_name,
                plugin_display_names: Vec::new(),
                connector_description,
            }
        })
        .collect();
    if server_name == CODEX_APPS_MCP_SERVER_NAME {
        return Ok(filter_disallowed_codex_apps_tools(tools));
    }
    Ok(tools)
}

fn sanitize_tool_connector_metadata(
    server_name: &str,
    tool: &mut RmcpTool,
    connector_id: Option<String>,
    connector_name: Option<String>,
    connector_description: Option<String>,
) -> (Option<String>, Option<String>, Option<String>) {
    if server_name == CODEX_APPS_MCP_SERVER_NAME {
        return (connector_id, connector_name, connector_description);
    }

    strip_untrusted_connector_meta(tool);
    (None, None, None)
}

fn strip_untrusted_connector_meta(tool: &mut RmcpTool) {
    if let Some(meta) = tool.meta.as_mut() {
        meta.retain(|key, _| !is_untrusted_connector_meta_key(key));
    }
}

fn is_untrusted_connector_meta_key(key: &str) -> bool {
    UNTRUSTED_CONNECTOR_META_KEYS.contains(&key)
}

fn resolve_bearer_token(
    server_name: &str,
    bearer_token_env_var: Option<&str>,
) -> Result<Option<String>> {
    let Some(env_var) = bearer_token_env_var else {
        return Ok(None);
    };

    match env::var(env_var) {
        Ok(value) => {
            if value.is_empty() {
                Err(anyhow!(
                    "Environment variable {env_var} for MCP server '{server_name}' is empty"
                ))
            } else {
                Ok(Some(value))
            }
        }
        Err(env::VarError::NotPresent) => Err(anyhow!(
            "Environment variable {env_var} for MCP server '{server_name}' is not set"
        )),
        Err(env::VarError::NotUnicode(_)) => Err(anyhow!(
            "Environment variable {env_var} for MCP server '{server_name}' contains invalid Unicode"
        )),
    }
}

fn validate_mcp_server_name(server_name: &str) -> Result<()> {
    let re = regex_lite::Regex::new(r"^[a-zA-Z0-9_-]+$")?;
    if !re.is_match(server_name) {
        return Err(anyhow!(
            "Invalid MCP server name '{server_name}': must match pattern {pattern}",
            pattern = re.as_str()
        ));
    }
    Ok(())
}

async fn start_server_task(
    server_name: String,
    client: Arc<RmcpClient>,
    params: StartServerTaskParams,
) -> Result<ManagedClient, StartupOutcomeError> {
    let StartServerTaskParams {
        startup_timeout,
        tool_timeout,
        tool_filter,
        channel_config,
        tx_event,
        elicitation_requests,
        codex_apps_tools_cache_context,
    } = params;
    let elicitation = elicitation_capability_for_server(&server_name);
    let params = InitializeRequestParams {
        meta: None,
        capabilities: ClientCapabilities {
            experimental: channel_capabilities(&channel_config),
            extensions: None,
            roots: None,
            sampling: None,
            elicitation,
            tasks: None,
        },
        client_info: Implementation {
            name: "codex-mcp-client".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            title: Some("Codex".into()),
            description: None,
            icons: None,
            website_url: None,
        },
        protocol_version: ProtocolVersion::V_2025_06_18,
    };

    let send_elicitation = elicitation_requests.make_sender(server_name.clone(), tx_event.clone());
    let send_custom_notification =
        make_custom_notification_sender(server_name.clone(), channel_config, tx_event.clone());

    let initialize_result = client
        .initialize(
            params,
            startup_timeout,
            send_elicitation,
            send_custom_notification,
        )
        .await
        .map_err(StartupOutcomeError::from)?;

    let server_supports_sandbox_state_meta_capability = initialize_result
        .capabilities
        .experimental
        .as_ref()
        .and_then(|exp| exp.get(MCP_SANDBOX_STATE_META_CAPABILITY))
        .is_some();
    let list_start = Instant::now();
    let fetch_start = Instant::now();
    let tools = list_tools_for_client_uncached(
        &server_name,
        &client,
        startup_timeout,
        initialize_result.instructions.as_deref(),
    )
    .await
    .map_err(StartupOutcomeError::from)?;
    emit_duration(
        MCP_TOOLS_FETCH_UNCACHED_DURATION_METRIC,
        fetch_start.elapsed(),
        &[],
    );
    write_cached_codex_apps_tools_if_needed(
        &server_name,
        codex_apps_tools_cache_context.as_ref(),
        &tools,
    );
    if server_name == CODEX_APPS_MCP_SERVER_NAME {
        emit_duration(
            MCP_TOOLS_LIST_DURATION_METRIC,
            list_start.elapsed(),
            &[("cache", "miss")],
        );
    }
    let tools = filter_tools(tools, &tool_filter);

    let managed = ManagedClient {
        client: Arc::clone(&client),
        tools,
        tool_timeout: Some(tool_timeout),
        tool_filter,
        server_instructions: initialize_result.instructions,
        server_supports_sandbox_state_meta_capability,
        codex_apps_tools_cache_context,
    };

    Ok(managed)
}

fn make_custom_notification_sender(
    server_name: String,
    channel_config: McpServerChannelConfig,
    tx_event: Sender<Event>,
) -> SendCustomNotification {
    let state = Arc::new(Mutex::new(ChannelDeliveryState::new(
        channel_config.dedupe_capacity,
        channel_config.queue_capacity,
    )));
    Box::new(move |method, params| {
        let server_name = server_name.clone();
        let channel_config = channel_config.clone();
        let state = Arc::clone(&state);
        let tx_event = tx_event.clone();
        async move {
            let Some(event) = channel_message_event(&server_name, &method, params) else {
                return;
            };
            let decision = {
                let mut state = match state.lock() {
                    Ok(state) => state,
                    Err(err) => {
                        warn!("failed to lock MCP channel delivery state: {err}");
                        return;
                    }
                };
                state.accept(&channel_config, &event)
            };
            match decision {
                ChannelDeliveryDecision::Accept => {}
                ChannelDeliveryDecision::Disabled => {
                    warn!(
                        server = %server_name,
                        method = %method,
                        "ignored MCP channel notification because this server has channel.enabled=false"
                    );
                    return;
                }
                ChannelDeliveryDecision::Duplicate => {
                    warn!(
                        server = %server_name,
                        channel_message_id = %event.id,
                        "ignored duplicate MCP channel notification"
                    );
                    return;
                }
                ChannelDeliveryDecision::QueueFull => {
                    warn!(
                        server = %server_name,
                        "ignored MCP channel notification because the channel queue is full"
                    );
                    return;
                }
                ChannelDeliveryDecision::RateLimited => {
                    warn!(
                        server = %server_name,
                        "ignored MCP channel notification because the channel rate limit was reached"
                    );
                    return;
                }
            }
            if let Err(err) = tx_event
                .send(Event {
                    id: format!("mcp_channel_{}", event.id),
                    msg: EventMsg::ChannelMessage(event),
                })
                .await
            {
                warn!("failed to forward MCP channel notification: {err}");
            }
        }
        .boxed()
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChannelDeliveryDecision {
    Accept,
    Disabled,
    Duplicate,
    QueueFull,
    RateLimited,
}

struct ChannelDeliveryState {
    dedupe_capacity: usize,
    seen_order: VecDeque<String>,
    seen: HashSet<String>,
    accepted_times: VecDeque<Instant>,
    burst_times: VecDeque<Instant>,
}

impl ChannelDeliveryState {
    fn new(dedupe_capacity: usize, _queue_capacity: usize) -> Self {
        Self {
            dedupe_capacity,
            seen_order: VecDeque::new(),
            seen: HashSet::new(),
            accepted_times: VecDeque::new(),
            burst_times: VecDeque::new(),
        }
    }

    fn accept(
        &mut self,
        config: &McpServerChannelConfig,
        event: &ChannelMessageEvent,
    ) -> ChannelDeliveryDecision {
        if !config.enabled {
            return ChannelDeliveryDecision::Disabled;
        }
        if self.seen.contains(&event.id) {
            return ChannelDeliveryDecision::Duplicate;
        }

        let now = Instant::now();
        let burst_window = Duration::from_secs(10);
        while self
            .burst_times
            .front()
            .is_some_and(|accepted_at| now.duration_since(*accepted_at) >= burst_window)
        {
            self.burst_times.pop_front();
        }
        if config.queue_capacity > 0 && self.burst_times.len() >= config.queue_capacity {
            return ChannelDeliveryDecision::QueueFull;
        }

        let rate_window = Duration::from_secs(60);
        while self
            .accepted_times
            .front()
            .is_some_and(|accepted_at| now.duration_since(*accepted_at) >= rate_window)
        {
            self.accepted_times.pop_front();
        }
        if config.rate_limit_per_minute > 0
            && self.accepted_times.len() >= config.rate_limit_per_minute as usize
        {
            return ChannelDeliveryDecision::RateLimited;
        }

        if self.dedupe_capacity > 0 {
            self.seen.insert(event.id.clone());
            self.seen_order.push_back(event.id.clone());
            while self.seen_order.len() > self.dedupe_capacity {
                if let Some(expired) = self.seen_order.pop_front() {
                    self.seen.remove(&expired);
                }
            }
        }
        self.accepted_times.push_back(now);
        self.burst_times.push_back(now);
        ChannelDeliveryDecision::Accept
    }
}

fn channel_capabilities(
    channel_config: &McpServerChannelConfig,
) -> Option<BTreeMap<String, JsonObject>> {
    if !channel_config.enabled {
        return None;
    }

    let mut value = JsonObject::new();
    value.insert("schemaVersion".to_string(), serde_json::json!(1));
    value.insert(
        "mode".to_string(),
        serde_json::json!(channel_mode_name(channel_config.mode)),
    );
    value.insert(
        "queueCapacity".to_string(),
        serde_json::json!(channel_config.queue_capacity),
    );
    value.insert(
        "dedupeCapacity".to_string(),
        serde_json::json!(channel_config.dedupe_capacity),
    );
    value.insert(
        "rateLimitPerMinute".to_string(),
        serde_json::json!(channel_config.rate_limit_per_minute),
    );

    Some(BTreeMap::from([(
        MCP_CHANNEL_NOTIFICATIONS_CAPABILITY.to_string(),
        value,
    )]))
}

fn channel_mode_name(mode: McpServerChannelMode) -> &'static str {
    match mode {
        McpServerChannelMode::Ask => "ask",
        McpServerChannelMode::Queue => "queue",
        McpServerChannelMode::Immediate => "immediate",
        McpServerChannelMode::Context => "context",
    }
}

fn channel_message_event(
    server_name: &str,
    method: &str,
    params: Option<Value>,
) -> Option<ChannelMessageEvent> {
    if !matches!(
        method,
        "notifications/codex/channel" | "notifications/claude/channel" | "notifications/channel"
    ) {
        return None;
    }

    let params = params.unwrap_or(Value::Null);
    let (text, source, sender, id, schema_version, metadata) = match params {
        Value::String(text) => (text, None, None, None, None, None),
        Value::Object(mut object) => {
            let text =
                take_string(&mut object, "text").or_else(|| take_string(&mut object, "message"))?;
            let source =
                take_string(&mut object, "source").or_else(|| take_string(&mut object, "channel"));
            let sender = take_string(&mut object, "sender")
                .or_else(|| take_string(&mut object, "sender_id"))
                .or_else(|| take_string(&mut object, "from"));
            let id = take_string(&mut object, "id")
                .or_else(|| take_string(&mut object, "message_id"))
                .or_else(|| {
                    let chat_id = object.get("chat_id")?;
                    let message_id = object.get("telegram_message_id")?;
                    Some(format!("{chat_id}:{message_id}"))
                });
            let schema_version = take_u32(&mut object, "schema_version")
                .or_else(|| take_u32(&mut object, "schemaVersion"));
            let metadata = (!object.is_empty()).then_some(Value::Object(object));
            (text, source, sender, id, schema_version, metadata)
        }
        _ => return None,
    };

    if text.trim().is_empty() {
        return None;
    }

    Some(ChannelMessageEvent {
        id: id.unwrap_or_else(|| fallback_channel_message_id(server_name, method, &text)),
        schema_version: schema_version.unwrap_or(1),
        server: server_name.to_string(),
        source,
        sender,
        text,
        metadata,
    })
}

fn fallback_channel_message_id(server_name: &str, method: &str, text: &str) -> String {
    let mut hasher = DefaultHasher::new();
    server_name.hash(&mut hasher);
    method.hash(&mut hasher);
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn take_string(object: &mut serde_json::Map<String, Value>, key: &str) -> Option<String> {
    match object.remove(key) {
        Some(Value::String(value)) if !value.is_empty() => Some(value),
        Some(value) if !value.is_null() => Some(value.to_string()),
        _ => None,
    }
}

fn take_u32(object: &mut serde_json::Map<String, Value>, key: &str) -> Option<u32> {
    match object.remove(key) {
        Some(Value::Number(value)) => value.as_u64().and_then(|value| u32::try_from(value).ok()),
        Some(Value::String(value)) => value.parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod channel_notification_tests {
    use super::ChannelDeliveryDecision;
    use super::ChannelDeliveryState;
    use super::channel_message_event;
    use codex_config::McpServerChannelConfig;
    use serde_json::json;

    #[test]
    fn parses_codex_channel_notification() {
        let event = channel_message_event(
            "telegram-channel",
            "notifications/codex/channel",
            Some(json!({
                "source": "telegram",
                "text": "hello",
                "sender": 42,
                "chat_id": "1001"
            })),
        )
        .expect("expected channel event");

        assert_eq!(event.server, "telegram-channel");
        assert!(!event.id.is_empty());
        assert_eq!(event.schema_version, 1);
        assert_eq!(event.source.as_deref(), Some("telegram"));
        assert_eq!(event.sender.as_deref(), Some("42"));
        assert_eq!(event.text, "hello");
        assert_eq!(
            event
                .metadata
                .as_ref()
                .and_then(|value| value.get("chat_id"))
                .and_then(|value| value.as_str()),
            Some("1001")
        );
    }

    #[test]
    fn ignores_unrelated_custom_notification() {
        assert!(
            channel_message_event(
                "telegram-channel",
                "notifications/message",
                Some(json!({"text": "hello"})),
            )
            .is_none()
        );
    }

    #[test]
    fn channel_delivery_defaults_to_disabled() {
        let event = channel_message_event(
            "telegram-channel",
            "notifications/codex/channel",
            Some(json!({"text": "hello", "id": "m1"})),
        )
        .expect("expected channel event");
        let mut state = ChannelDeliveryState::new(10, 10);

        assert_eq!(
            state.accept(&McpServerChannelConfig::default(), &event),
            ChannelDeliveryDecision::Disabled
        );
    }

    #[test]
    fn channel_delivery_deduplicates_message_ids() {
        let config = McpServerChannelConfig {
            enabled: true,
            ..Default::default()
        };
        let event = channel_message_event(
            "telegram-channel",
            "notifications/codex/channel",
            Some(json!({"text": "hello", "id": "m1"})),
        )
        .expect("expected channel event");
        let mut state = ChannelDeliveryState::new(10, 10);

        assert_eq!(
            state.accept(&config, &event),
            ChannelDeliveryDecision::Accept
        );
        assert_eq!(
            state.accept(&config, &event),
            ChannelDeliveryDecision::Duplicate
        );
    }

    #[test]
    fn channel_delivery_applies_rate_limit() {
        let config = McpServerChannelConfig {
            enabled: true,
            rate_limit_per_minute: 1,
            ..Default::default()
        };
        let first = channel_message_event(
            "telegram-channel",
            "notifications/codex/channel",
            Some(json!({"text": "hello", "id": "m1"})),
        )
        .expect("expected channel event");
        let second = channel_message_event(
            "telegram-channel",
            "notifications/codex/channel",
            Some(json!({"text": "world", "id": "m2"})),
        )
        .expect("expected channel event");
        let mut state = ChannelDeliveryState::new(10, 10);

        assert_eq!(
            state.accept(&config, &first),
            ChannelDeliveryDecision::Accept
        );
        assert_eq!(
            state.accept(&config, &second),
            ChannelDeliveryDecision::RateLimited
        );
    }
}

struct StartServerTaskParams {
    startup_timeout: Option<Duration>, // TODO: cancel_token should handle this.
    tool_timeout: Duration,
    tool_filter: ToolFilter,
    channel_config: McpServerChannelConfig,
    tx_event: Sender<Event>,
    elicitation_requests: ElicitationRequestManager,
    codex_apps_tools_cache_context: Option<CodexAppsToolsCacheContext>,
}

async fn make_rmcp_client(
    server_name: &str,
    config: McpServerConfig,
    store_mode: OAuthCredentialsStoreMode,
    runtime_environment: McpRuntimeEnvironment,
    runtime_auth_provider: Option<SharedAuthProvider>,
) -> Result<RmcpClient, StartupOutcomeError> {
    let McpServerConfig {
        transport,
        experimental_environment,
        ..
    } = config;
    let remote_environment = match experimental_environment.as_deref() {
        None | Some("local") => false,
        Some("remote") => {
            if !runtime_environment.environment().is_remote() {
                return Err(StartupOutcomeError::from(anyhow!(
                    "remote MCP server `{server_name}` requires a remote environment"
                )));
            }
            true
        }
        Some(environment) => {
            return Err(StartupOutcomeError::from(anyhow!(
                "unsupported experimental_environment `{environment}` for MCP server `{server_name}`"
            )));
        }
    };

    match transport {
        McpServerTransportConfig::Stdio {
            command,
            args,
            env,
            env_vars,
            cwd,
        } => {
            let command_os: OsString = command.into();
            let args_os: Vec<OsString> = args.into_iter().map(Into::into).collect();
            let env_os = env.map(|env| {
                env.into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect::<HashMap<_, _>>()
            });
            let launcher = if remote_environment {
                Arc::new(ExecutorStdioServerLauncher::new(
                    runtime_environment.environment().get_exec_backend(),
                    runtime_environment.fallback_cwd(),
                ))
            } else {
                Arc::new(LocalStdioServerLauncher::new(
                    runtime_environment.fallback_cwd(),
                )) as Arc<dyn StdioServerLauncher>
            };

            // `RmcpClient` always sees a launched MCP stdio server. The
            // launcher hides whether that means a local child process or an
            // executor process whose stdin/stdout bytes cross the process API.
            RmcpClient::new_stdio_client(command_os, args_os, env_os, &env_vars, cwd, launcher)
                .await
                .map_err(|err| StartupOutcomeError::from(anyhow!(err)))
        }
        McpServerTransportConfig::StreamableHttp {
            url,
            http_headers,
            env_http_headers,
            bearer_token_env_var,
        } => {
            let http_client: Arc<dyn HttpClient> = if remote_environment {
                runtime_environment.environment().get_http_client()
            } else {
                Arc::new(ReqwestHttpClient)
            };
            let resolved_bearer_token =
                match resolve_bearer_token(server_name, bearer_token_env_var.as_deref()) {
                    Ok(token) => token,
                    Err(error) => return Err(error.into()),
                };
            RmcpClient::new_streamable_http_client(
                server_name,
                &url,
                resolved_bearer_token,
                http_headers,
                env_http_headers,
                store_mode,
                http_client,
                runtime_auth_provider,
            )
            .await
            .map_err(StartupOutcomeError::from)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::JsonObject;
    use rmcp::model::Meta;

    fn tool_with_connector_meta() -> RmcpTool {
        RmcpTool {
            name: "capture_file_upload".to_string().into(),
            title: None,
            description: Some("test tool".to_string().into()),
            input_schema: Arc::new(JsonObject::default()),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: Some(Meta(
                serde_json::json!({
                    "connector_id": "connector_gmail",
                    "connector_name": "Gmail",
                    "connector_display_name": "Gmail",
                    "connector_description": "Mail connector",
                    "connectorDescription": "Mail connector",
                    "connectorFutureField": "future connector metadata",
                    "CONNECTOR_UPPERCASE": "uppercase connector metadata",
                    "openai/fileParams": ["file"],
                    "custom": "kept"
                })
                .as_object()
                .expect("object")
                .clone(),
            )),
        }
    }

    #[test]
    fn custom_mcp_connector_metadata_is_stripped() {
        let mut tool = tool_with_connector_meta();

        let (connector_id, connector_name, connector_description) =
            sanitize_tool_connector_metadata(
                "minimaltest",
                &mut tool,
                Some("connector_gmail".to_string()),
                Some("Gmail".to_string()),
                Some("Mail connector".to_string()),
            );

        assert_eq!(connector_id, None);
        assert_eq!(connector_name, None);
        assert_eq!(connector_description, None);

        let meta = tool.meta.as_ref().expect("meta");
        for key in [
            "connector_id",
            "connector_name",
            "connector_display_name",
            "connector_description",
            "connectorDescription",
        ] {
            assert!(!meta.0.contains_key(key), "{key} should be stripped");
        }
        assert!(meta.0.contains_key("connectorFutureField"));
        assert!(meta.0.contains_key("CONNECTOR_UPPERCASE"));
        assert!(meta.0.contains_key("openai/fileParams"));
        assert_eq!(
            meta.0.get("custom").and_then(|value| value.as_str()),
            Some("kept")
        );
    }

    #[test]
    fn codex_apps_connector_metadata_is_preserved() {
        let mut tool = tool_with_connector_meta();

        let (connector_id, connector_name, connector_description) =
            sanitize_tool_connector_metadata(
                CODEX_APPS_MCP_SERVER_NAME,
                &mut tool,
                Some("connector_gmail".to_string()),
                Some("Gmail".to_string()),
                Some("Mail connector".to_string()),
            );

        assert_eq!(connector_id.as_deref(), Some("connector_gmail"));
        assert_eq!(connector_name.as_deref(), Some("Gmail"));
        assert_eq!(connector_description.as_deref(), Some("Mail connector"));

        let meta = tool.meta.as_ref().expect("meta");
        for key in [
            "connector_id",
            "connector_name",
            "connector_display_name",
            "connector_description",
            "connectorDescription",
            "connectorFutureField",
            "CONNECTOR_UPPERCASE",
        ] {
            assert!(meta.0.contains_key(key), "{key} should be preserved");
        }
    }
}
