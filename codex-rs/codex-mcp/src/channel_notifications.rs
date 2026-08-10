use std::collections::BTreeMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use async_channel::Sender;
use codex_config::McpServerChannelConfig;
use codex_config::McpServerChannelMode;
use codex_protocol::protocol::ChannelMessageAttachment;
use codex_protocol::protocol::ChannelMessageEvent;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_rmcp_client::SendCustomNotification;
use futures::FutureExt;
use rmcp::model::JsonObject;
use serde_json::Value;
use tracing::warn;

pub(crate) const MCP_CHANNEL_NOTIFICATIONS_CAPABILITY: &str = "codex/channel-notifications";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChannelDeliveryDecision {
    Accept,
    Disabled,
    Duplicate,
    QueueFull,
    RateLimited,
}

pub(crate) struct ChannelDeliveryState {
    dedupe_capacity: usize,
    seen_order: VecDeque<String>,
    seen: HashSet<String>,
    accepted_times: VecDeque<Instant>,
    burst_times: VecDeque<Instant>,
}

impl ChannelDeliveryState {
    pub(crate) fn new(dedupe_capacity: usize) -> Self {
        Self {
            dedupe_capacity,
            seen_order: VecDeque::new(),
            seen: HashSet::new(),
            accepted_times: VecDeque::new(),
            burst_times: VecDeque::new(),
        }
    }

    pub(crate) fn accept(
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

pub(crate) fn channel_capabilities(
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

pub(crate) fn make_custom_notification_sender(
    server_name: String,
    channel_config: McpServerChannelConfig,
    tx_event: Option<Sender<Event>>,
) -> SendCustomNotification {
    let state = Arc::new(Mutex::new(ChannelDeliveryState::new(
        channel_config.dedupe_capacity,
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
            let Some(tx_event) = tx_event else {
                warn!(
                    server = %server_name,
                    "failed to forward MCP channel notification because no event receiver is attached"
                );
                return;
            };
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

pub(crate) fn channel_message_event(
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
    let (text, source, sender, id, schema_version, attachments, metadata) = match params {
        Value::String(text) => (text, None, None, None, None, Vec::new(), None),
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
            let attachments = take_channel_attachments(&mut object);
            let metadata = (!object.is_empty()).then_some(Value::Object(object));
            (
                text,
                source,
                sender,
                id,
                schema_version,
                attachments,
                metadata,
            )
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
        attachments,
        metadata,
    })
}

fn take_channel_attachments(
    object: &mut serde_json::Map<String, Value>,
) -> Vec<ChannelMessageAttachment> {
    let Some(Value::Array(items)) = object.remove("attachments") else {
        return Vec::new();
    };

    items
        .into_iter()
        .filter_map(|item| {
            let Value::Object(mut object) = item else {
                return None;
            };
            let kind = take_string(&mut object, "kind")
                .or_else(|| take_string(&mut object, "type"))
                .unwrap_or_else(|| "file".to_string());
            let path = take_string(&mut object, "path").map(std::path::PathBuf::from);
            let mime_type = take_string(&mut object, "mime_type")
                .or_else(|| take_string(&mut object, "mimeType"));
            let file_name = take_string(&mut object, "file_name")
                .or_else(|| take_string(&mut object, "fileName"));
            let file_size =
                take_u64(&mut object, "file_size").or_else(|| take_u64(&mut object, "fileSize"));
            let metadata = (!object.is_empty()).then_some(Value::Object(object));
            Some(ChannelMessageAttachment {
                kind,
                path,
                mime_type,
                file_name,
                file_size,
                metadata,
            })
        })
        .collect()
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

fn take_u64(object: &mut serde_json::Map<String, Value>, key: &str) -> Option<u64> {
    match object.remove(key) {
        Some(Value::Number(value)) => value.as_u64(),
        Some(Value::String(value)) => value.parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_config::McpServerChannelConfig;
    use codex_config::McpServerChannelMode;
    use codex_protocol::protocol::EventMsg;
    use serde_json::json;

    #[tokio::test]
    async fn accepted_notification_sender_emits_protocol_event() {
        let (tx_event, rx_event) = async_channel::unbounded();
        let sender = make_custom_notification_sender(
            "telegram-channel".to_string(),
            McpServerChannelConfig {
                enabled: true,
                ..Default::default()
            },
            Some(tx_event),
        );

        sender(
            "notifications/codex/channel".to_string(),
            Some(json!({
                "id": "telegram:101",
                "source": "telegram",
                "sender": "alice",
                "text": "ship it"
            })),
        )
        .await;

        let event = rx_event.recv().await.expect("channel event");
        assert_eq!(event.id, "mcp_channel_telegram:101");
        let EventMsg::ChannelMessage(message) = event.msg else {
            panic!("expected channel message event");
        };
        assert_eq!(message.server, "telegram-channel");
        assert_eq!(message.source.as_deref(), Some("telegram"));
        assert_eq!(message.sender.as_deref(), Some("alice"));
        assert_eq!(message.text, "ship it");
    }

    #[test]
    fn parses_channel_notification_with_routing_metadata() {
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
        assert_eq!(event.metadata, Some(json!({"chat_id": "1001"})));
    }

    #[test]
    fn parses_image_attachment() {
        let event = channel_message_event(
            "telegram-channel",
            "notifications/codex/channel",
            Some(json!({
                "text": "photo",
                "attachments": [{
                    "kind": "photo",
                    "path": "/tmp/photo.jpg",
                    "mime_type": "image/jpeg",
                    "file_name": "photo.jpg",
                    "file_size": 123
                }]
            })),
        )
        .expect("expected channel event");

        assert_eq!(event.attachments.len(), 1);
        assert_eq!(event.attachments[0].kind, "photo");
        assert_eq!(
            event.attachments[0].mime_type.as_deref(),
            Some("image/jpeg")
        );
        assert_eq!(event.attachments[0].file_size, Some(123));
    }

    #[test]
    fn ignores_unrelated_and_empty_notifications() {
        assert!(
            channel_message_event(
                "telegram-channel",
                "notifications/message",
                Some(json!({"text": "hello"})),
            )
            .is_none()
        );
        assert!(
            channel_message_event(
                "telegram-channel",
                "notifications/codex/channel",
                Some(json!({"text": "  "})),
            )
            .is_none()
        );
    }

    #[test]
    fn channel_policy_defaults_to_disabled_and_deduplicates_when_enabled() {
        let event = channel_message_event(
            "telegram-channel",
            "notifications/codex/channel",
            Some(json!({"text": "hello", "id": "m1"})),
        )
        .expect("expected channel event");
        let mut state = ChannelDeliveryState::new(10);

        assert_eq!(
            state.accept(&McpServerChannelConfig::default(), &event),
            ChannelDeliveryDecision::Disabled
        );
        let enabled = McpServerChannelConfig {
            enabled: true,
            ..Default::default()
        };
        assert_eq!(
            state.accept(&enabled, &event),
            ChannelDeliveryDecision::Accept
        );
        assert_eq!(
            state.accept(&enabled, &event),
            ChannelDeliveryDecision::Duplicate
        );
    }

    #[test]
    fn channel_policy_applies_burst_and_per_minute_limits() {
        let first = channel_message_event(
            "telegram-channel",
            "notifications/codex/channel",
            Some(json!({"text": "one", "id": "m1"})),
        )
        .expect("first event");
        let second = channel_message_event(
            "telegram-channel",
            "notifications/codex/channel",
            Some(json!({"text": "two", "id": "m2"})),
        )
        .expect("second event");

        let burst_config = McpServerChannelConfig {
            enabled: true,
            queue_capacity: 1,
            rate_limit_per_minute: 30,
            ..Default::default()
        };
        let mut burst_state = ChannelDeliveryState::new(10);
        assert_eq!(
            burst_state.accept(&burst_config, &first),
            ChannelDeliveryDecision::Accept
        );
        assert_eq!(
            burst_state.accept(&burst_config, &second),
            ChannelDeliveryDecision::QueueFull
        );

        let rate_config = McpServerChannelConfig {
            enabled: true,
            queue_capacity: 10,
            rate_limit_per_minute: 1,
            ..Default::default()
        };
        let mut rate_state = ChannelDeliveryState::new(10);
        assert_eq!(
            rate_state.accept(&rate_config, &first),
            ChannelDeliveryDecision::Accept
        );
        assert_eq!(
            rate_state.accept(&rate_config, &second),
            ChannelDeliveryDecision::RateLimited
        );
    }

    #[test]
    fn enabled_channel_capability_advertises_actual_mode_and_limits() {
        let config = McpServerChannelConfig {
            enabled: true,
            mode: McpServerChannelMode::Immediate,
            queue_capacity: 10,
            dedupe_capacity: 25,
            rate_limit_per_minute: 6,
        };

        assert_eq!(
            channel_capabilities(&config),
            Some(std::collections::BTreeMap::from([(
                MCP_CHANNEL_NOTIFICATIONS_CAPABILITY.to_string(),
                serde_json::Map::from_iter([
                    ("schemaVersion".to_string(), json!(1)),
                    ("mode".to_string(), json!("immediate")),
                    ("queueCapacity".to_string(), json!(10)),
                    ("dedupeCapacity".to_string(), json!(25)),
                    ("rateLimitPerMinute".to_string(), json!(6)),
                ]),
            )]))
        );
    }
}
