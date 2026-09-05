use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::context::boxed_tool_output;
use crate::tools::handlers::dynamic::DynamicToolResponseWait;
use crate::tools::handlers::dynamic::request_dynamic_tool;
use crate::tools::handlers::loop_control_spec::LOOP_CONTROL_TOOL_NAME;
use crate::tools::handlers::loop_control_spec::create_loop_control_tool;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::CoreToolRuntime;
use crate::tools::registry::ToolExecutor;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::protocol::LoopControlAction;
use codex_protocol::protocol::LoopControlEvent;
use codex_protocol::protocol::LoopControlMode;
use codex_tools::ToolName;
use codex_tools::ToolSpec;

pub struct LoopControlHandler;

impl ToolExecutor<ToolInvocation> for LoopControlHandler {
    fn tool_name(&self) -> ToolName {
        ToolName::plain(LOOP_CONTROL_TOOL_NAME)
    }

    fn spec(&self) -> ToolSpec {
        create_loop_control_tool()
    }

    fn handle<'a>(&'a self, invocation: ToolInvocation) -> codex_tools::ToolExecutorFuture<'a>
    where
        ToolInvocation: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                payload,
                call_id,
                ..
            } = invocation;

            let arguments = match payload {
                ToolPayload::Function { arguments } => arguments,
                _ => {
                    return Err(FunctionCallError::RespondToModel(format!(
                        "{LOOP_CONTROL_TOOL_NAME} handler received unsupported payload"
                    )));
                }
            };

            if turn.session_source.is_non_root_agent() {
                return Err(FunctionCallError::RespondToModel(
                    "loop_control can only be used by the root thread".to_string(),
                ));
            }

            let event: LoopControlEvent = parse_arguments(&arguments)?;
            validate_loop_control(&event)?;
            let mut arguments = serde_json::to_value(event)
                .map_err(|error| FunctionCallError::RespondToModel(error.to_string()))?;
            let deadline_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .saturating_add(5000);
            arguments["_loop_deadline_ms"] = serde_json::json!(deadline_ms as u64);
            let response = request_dynamic_tool(&session, turn.as_ref(), call_id, self.tool_name(), arguments, DynamicToolResponseWait::Timeout(std::time::Duration::from_secs(5)))
                .await.ok_or_else(|| FunctionCallError::RespondToModel("Local loop control requires an attached TUI; its response timed out or was cancelled.".to_string()))?;
            Ok(boxed_tool_output(FunctionToolOutput::from_content(
                response
                    .content_items
                    .into_iter()
                    .map(FunctionCallOutputContentItem::from)
                    .collect(),
                Some(response.success),
            )))
        })
    }
}

impl CoreToolRuntime for LoopControlHandler {}

fn validate_loop_control(event: &LoopControlEvent) -> Result<(), FunctionCallError> {
    if event
        .prompt
        .as_ref()
        .is_some_and(|prompt| prompt.len() > 1024)
        || event
            .reason
            .as_ref()
            .is_some_and(|reason| reason.len() > 512)
    {
        return Err(FunctionCallError::RespondToModel(
            "Loop prompt or reason exceeds its size limit.".to_string(),
        ));
    }
    if event.action == LoopControlAction::Start {
        if event.max_iterations == Some(0) {
            return Err(FunctionCallError::RespondToModel(
                "Loop max must be greater than zero.".to_string(),
            ));
        }
        if event.mode == Some(LoopControlMode::Timed)
            && !event
                .interval_minutes
                .is_some_and(|minutes| (1..=525_600).contains(&minutes))
        {
            return Err(FunctionCallError::RespondToModel(
                "Timed loops require interval_minutes between 1 and 525600.".to_string(),
            ));
        }
    }
    Ok(())
}
