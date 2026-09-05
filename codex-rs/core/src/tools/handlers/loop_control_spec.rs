//! Responses API tool definition for the local `/loop` runner.

use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use serde_json::json;
use std::collections::BTreeMap;

pub const LOOP_CONTROL_TOOL_NAME: &str = "loop_control";

pub fn create_loop_control_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "action".to_string(),
            JsonSchema::string_enum(
                vec![json!("start"), json!("stop"), json!("status")],
                Some("Loop control action to apply.".to_string()),
            ),
        ),
        (
            "mode".to_string(),
            JsonSchema::string_enum(
                vec![json!("timed"), json!("immediate"), json!("once")],
                Some("Loop mode for start actions.".to_string()),
            ),
        ),
        (
            "interval_minutes".to_string(),
            JsonSchema::integer(Some(
                "Interval from 1 to 525600 minutes for timed loop mode.".to_string(),
            )),
        ),
        (
            "max_iterations".to_string(),
            JsonSchema::integer(Some(
                "Optional positive maximum number of loop iterations.".to_string(),
            )),
        ),
        (
            "prompt".to_string(),
            JsonSchema::string(Some(
                "Autonomous follow-up prompt for start actions, at most 1024 UTF-8 bytes."
                    .to_string(),
            )),
        ),
        (
            "reason".to_string(),
            JsonSchema::string(Some(
                "Short reason for the loop control request, at most 512 UTF-8 bytes.".to_string(),
            )),
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: LOOP_CONTROL_TOOL_NAME.to_string(),
        description: r#"Controls the local Codex loop runner.
Use this tool, not a printed slash command, to start, stop, or inspect `/loop`.
For immediate autonomous continuation use action=start, mode=immediate, prompt=...
For timed continuation use action=start, mode=timed, interval_minutes=N, prompt=...
The tool requires an attached local TUI and returns that TUI's authoritative loop status after applying the request. The runner belongs to that active TUI thread.
Stop only when the user requests it or the explicit loop stop condition is met. Native goal completion or blockage alone must not stop this independent loop."#
            .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["action".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}
