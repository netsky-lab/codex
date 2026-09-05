use super::*;
use codex_protocol::protocol::ChannelMessageEvent;
use pretty_assertions::assert_eq;
use serde_json::json;

const MAX_CHANNEL_CONTEXT_BYTES: usize = 8 * 1024;

fn event() -> ChannelMessageEvent {
    ChannelMessageEvent {
        id: "telegram:123:456".to_string(),
        schema_version: 1,
        server: "telegram-channel".to_string(),
        source: Some("telegram".to_string()),
        sender: Some("123".to_string()),
        text: "x".repeat(4096),
        attachments: Vec::new(),
        metadata: Some(json!({"chat_id": "123", "message_thread_id": 42})),
    }
}

#[test]
fn channel_context_preserves_maximum_ascii_telegram_text_and_routing() {
    let event = event();
    let context = ChannelMessageContext::new(&event).expect("bounded Telegram message");
    let rendered = context.render();
    let json = rendered
        .strip_prefix("Inbound channel message:\n```json\n")
        .expect("existing prefix")
        .strip_suffix("\n```")
        .expect("existing suffix");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(json).expect("valid JSON"),
        json!({
            "type": "channel_message",
            "schema_version": 1,
            "id": "telegram:123:456",
            "server": "telegram-channel",
            "source": "telegram",
            "sender": "123",
            "text": "x".repeat(4096),
            "attachments": [],
            "metadata": {"chat_id": "123", "message_thread_id": 42}
        })
    );
    assert!(rendered.len() <= MAX_CHANNEL_CONTEXT_BYTES);
}

#[test]
fn channel_context_bounds_final_render_including_escaped_nested_metadata() {
    let mut event = event();
    event.text = "hello".to_string();
    event.metadata = Some(json!({"route": {"chat_id": "123", "extra": "\"\\\n😀".repeat(100)}}));
    let initial_len = ChannelMessageContext::new(&event)
        .expect("bounded nested metadata")
        .render()
        .len();
    event
        .text
        .push_str(&"x".repeat(MAX_CHANNEL_CONTEXT_BYTES - initial_len));
    assert_eq!(
        ChannelMessageContext::new(&event)
            .expect("exact boundary")
            .render()
            .len(),
        MAX_CHANNEL_CONTEXT_BYTES
    );
    event.text.push('x');
    assert!(ChannelMessageContext::new(&event).is_none());

    event.text = "hello".to_string();
    event.metadata = Some(json!({"route": {"extra": "😀".repeat(MAX_CHANNEL_CONTEXT_BYTES)}}));
    assert!(ChannelMessageContext::new(&event).is_none());
}
