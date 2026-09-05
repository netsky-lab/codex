use anyhow::Result;
use codex_core::TurnInputRequest;
use codex_protocol::dynamic_tools::DynamicToolCallOutputContentItem;
use codex_protocol::dynamic_tools::DynamicToolResponse;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::user_input::UserInput;
use core_test_support::responses;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use pretty_assertions::assert_eq;
use serde_json::Value;
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loop_control_round_trips_authoritative_client_status() -> Result<()> {
    let server = responses::start_mock_server().await;
    let mock = responses::mount_sse_sequence(
        &server,
        vec![
            responses::sse(vec![
                responses::ev_function_call(
                    "loop-start",
                    "loop_control",
                    &json!({"action":"start", "mode":"once", "prompt":"inspect the project"})
                        .to_string(),
                ),
                responses::ev_completed("start-response"),
            ]),
            responses::sse(vec![
                responses::ev_function_call(
                    "loop-status",
                    "loop_control",
                    &json!({"action":"status"}).to_string(),
                ),
                responses::ev_completed("status-response"),
            ]),
            responses::sse(vec![
                responses::ev_assistant_message("done", "Request sent."),
                responses::ev_completed("done-response"),
            ]),
        ],
    )
    .await;
    let test = test_codex().build_with_auto_env(&server).await?;
    test.codex
        .start_or_steer_turn(TurnInputRequest::user_input(vec![UserInput::Text {
            text: "Run one local loop iteration and inspect its status.".to_string(),
            text_elements: Vec::new(),
        }]))
        .await?;
    let status = json!({"active": true, "mode":"once", "completed_iterations":0, "max_iterations":1, "timer_pending":false, "prompt":"inspect the project"});
    for (call_id, arguments) in [
        (
            "loop-start",
            json!({"action":"start", "mode":"once", "prompt":"inspect the project"}),
        ),
        ("loop-status", json!({"action":"status"})),
    ] {
        let event = wait_for_event(&test.codex, |event| {
            matches!(event, EventMsg::DynamicToolCallRequest(_))
        })
        .await;
        let EventMsg::DynamicToolCallRequest(mut request) = event else {
            unreachable!()
        };
        assert!(
            request
                .arguments
                .as_object_mut()
                .expect("arguments")
                .remove("_loop_deadline_ms")
                .is_some()
        );
        assert_eq!(
            (
                request.call_id.as_str(),
                request.namespace,
                request.tool,
                request.arguments
            ),
            (call_id, None, "loop_control".to_string(), arguments)
        );
        test.codex
            .submit(Op::DynamicToolResponse {
                id: request.call_id,
                response: DynamicToolResponse {
                    content_items: vec![DynamicToolCallOutputContentItem::InputText {
                        text: status.to_string(),
                    }],
                    success: true,
                },
            })
            .await?;
    }
    wait_for_event(&test.codex, |event| {
        matches!(event, EventMsg::TurnComplete(_))
    })
    .await;
    let requests = mock.requests();
    for (index, call_id) in [(1, "loop-start"), (2, "loop-status")] {
        let output: Value = serde_json::from_str(
            &requests[index]
                .function_call_output_content_and_success(call_id)
                .expect("tool response")
                .0
                .expect("text output"),
        )?;
        assert_eq!(output, status);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loop_control_without_attached_tui_times_out() -> Result<()> {
    let server = responses::start_mock_server().await;
    let mock = responses::mount_sse_sequence(
        &server,
        vec![
            responses::sse(vec![
                responses::ev_function_call(
                    "loop-status",
                    "loop_control",
                    "{\"action\":\"status\"}",
                ),
                responses::ev_completed("first"),
            ]),
            responses::sse(vec![
                responses::ev_assistant_message("done", "No local runner."),
                responses::ev_completed("second"),
            ]),
        ],
    )
    .await;
    let test = test_codex().build_with_auto_env(&server).await?;
    test.codex
        .start_or_steer_turn(TurnInputRequest::user_input(vec![UserInput::Text {
            text: "Check local loop status.".to_string(),
            text_elements: Vec::new(),
        }]))
        .await?;
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            if matches!(
                test.codex.next_event().await.expect("event").msg,
                EventMsg::TurnComplete(_)
            ) {
                break;
            }
        }
    })
    .await?;
    let requests = mock.requests();
    let output = requests[1]
        .function_call_output_content_and_success("loop-status")
        .expect("output")
        .0
        .expect("text");
    assert!(output.contains("requires an attached TUI"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loop_control_is_unavailable_to_subagents() -> Result<()> {
    let server = responses::start_mock_server().await;
    let mock = responses::mount_sse_sequence(
        &server,
        vec![
            responses::sse(vec![
                responses::ev_function_call(
                    "loop-status",
                    "loop_control",
                    "{\"action\":\"status\"}",
                ),
                responses::ev_completed("first"),
            ]),
            responses::sse(vec![
                responses::ev_assistant_message("done", "Unavailable."),
                responses::ev_completed("second"),
            ]),
        ],
    )
    .await;
    let mut test = test_codex().build_with_auto_env(&server).await?;
    test.codex.shutdown_and_wait().await?;
    let thread = test
        .thread_manager
        .start_thread(codex_core::StartThreadOptions {
            session_source: Some(codex_protocol::protocol::SessionSource::SubAgent(
                codex_protocol::protocol::SubAgentSource::Other("test".to_string()),
            )),
            ..codex_core::StartThreadOptions::new(test.config.clone())
        })
        .await?;
    test.codex = thread.thread;
    test.submit_text_turn("Check local loop status.").await?;
    let requests = mock.requests();
    let output = requests[1]
        .function_call_output_content_and_success("loop-status")
        .expect("output")
        .0
        .expect("text");
    assert_eq!(output, "unsupported call: loop_control");
    test.codex.shutdown_and_wait().await?;
    Ok(())
}
