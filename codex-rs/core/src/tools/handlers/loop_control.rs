use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::LoopControlEvent;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use codex_tools::LOOP_CONTROL_TOOL_NAME;
use codex_tools::create_loop_control_tool;

pub struct LoopControlHandler;

impl ToolHandler for LoopControlHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(LOOP_CONTROL_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_loop_control_tool())
    }

    fn kind(&self) -> ToolKind {
        ToolKind::Function
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
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
        session
            .send_event(turn.as_ref(), EventMsg::LoopControl(event))
            .await;

        Ok(FunctionToolOutput::from_text(
            "Loop control request submitted.".to_string(),
            Some(true),
        ))
    }
}
