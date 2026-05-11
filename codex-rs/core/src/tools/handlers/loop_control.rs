use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolHandler;
use crate::tools::registry::ToolKind;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::LoopControlAction;
use codex_protocol::protocol::LoopControlEvent;
use codex_protocol::protocol::LoopControlMode;
use codex_protocol::protocol::LoopStatusMode;
use codex_protocol::protocol::LoopStatusSnapshot;
use codex_protocol::protocol::get_loop_status_snapshot;
use codex_protocol::protocol::set_loop_status_snapshot;
use codex_tools::LOOP_CONTROL_TOOL_NAME;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
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
        let status = apply_loop_control_to_shared_status(&event)?;
        session
            .send_event(turn.as_ref(), EventMsg::LoopControl(event))
            .await;

        Ok(FunctionToolOutput::from_text(
            loop_status_output(&status),
            Some(true),
        ))
    }
}

fn apply_loop_control_to_shared_status(
    event: &LoopControlEvent,
) -> Result<LoopStatusSnapshot, FunctionCallError> {
    let mut status = get_loop_status_snapshot();
    match event.action {
        LoopControlAction::Status => {}
        LoopControlAction::Stop => {
            status.active = false;
            status.timer_pending = false;
            status.last_reason = event.reason.clone();
            set_loop_status_snapshot(status.clone());
        }
        LoopControlAction::Start => {
            if event.max_iterations == Some(0) {
                return Err(FunctionCallError::RespondToModel(
                    "Loop max must be greater than zero.".to_string(),
                ));
            }
            let mode = event.mode.clone().unwrap_or(LoopControlMode::Immediate);
            let interval_minutes = match mode {
                LoopControlMode::Timed => {
                    let Some(interval_minutes) = event.interval_minutes else {
                        return Err(FunctionCallError::RespondToModel(
                            "loop_control start mode=timed requires interval_minutes.".to_string(),
                        ));
                    };
                    if interval_minutes == 0 {
                        return Err(FunctionCallError::RespondToModel(
                            "Loop interval must be greater than zero minutes.".to_string(),
                        ));
                    }
                    Some(interval_minutes)
                }
                LoopControlMode::Immediate | LoopControlMode::Once => None,
            };
            status = LoopStatusSnapshot {
                active: true,
                mode: match mode {
                    LoopControlMode::Timed => LoopStatusMode::Timed,
                    LoopControlMode::Immediate => LoopStatusMode::Immediate,
                    LoopControlMode::Once => LoopStatusMode::Once,
                },
                completed_iterations: 0,
                interval_minutes,
                max_iterations: match mode {
                    LoopControlMode::Once => Some(1),
                    LoopControlMode::Timed | LoopControlMode::Immediate => event.max_iterations,
                },
                timer_pending: false,
                prompt: event
                    .prompt
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or_default()
                    .to_string(),
                last_reason: event.reason.clone(),
            };
            set_loop_status_snapshot(status.clone());
        }
    }
    Ok(status)
}

fn loop_status_output(status: &LoopStatusSnapshot) -> String {
    let payload = serde_json::json!({
        "type": "loop_status",
        "active": status.active,
        "mode": status.mode,
        "completed_iterations": status.completed_iterations,
        "interval_minutes": status.interval_minutes,
        "max_iterations": status.max_iterations,
        "timer_pending": status.timer_pending,
        "prompt": status.prompt,
        "last_reason": status.last_reason,
        "summary": status.summary(),
    });
    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| status.summary())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::protocol::LoopControlMode;

    #[test]
    fn start_returns_structured_loop_status() {
        let status = apply_loop_control_to_shared_status(&LoopControlEvent {
            action: LoopControlAction::Start,
            mode: Some(LoopControlMode::Immediate),
            interval_minutes: None,
            max_iterations: Some(3),
            prompt: Some("keep going".to_string()),
            reason: None,
        })
        .expect("start should be valid");
        let output = loop_status_output(&status);
        let value: serde_json::Value =
            serde_json::from_str(&output).expect("output should be json");

        assert_eq!(value["type"], "loop_status");
        assert_eq!(value["active"], true);
        assert_eq!(value["mode"], "immediate");
        assert_eq!(value["max_iterations"], 3);
        assert_eq!(value["prompt"], "keep going");
    }

    #[test]
    fn timed_start_requires_positive_interval() {
        let err = apply_loop_control_to_shared_status(&LoopControlEvent {
            action: LoopControlAction::Start,
            mode: Some(LoopControlMode::Timed),
            interval_minutes: Some(0),
            max_iterations: None,
            prompt: Some("keep going".to_string()),
            reason: None,
        })
        .expect_err("zero interval should fail");

        assert!(err.to_string().contains("greater than zero"));
    }
}
