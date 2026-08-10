//! Inbound channel-message formatting and submission behavior.

use super::*;
use crate::chatwidget::user_messages::UserMessageHistoryOverride;
use codex_protocol::models::local_image_label_text;
use codex_protocol::protocol::ChannelMessageAttachment;
use codex_protocol::protocol::ChannelMessageEvent;

pub(crate) fn format_channel_message_for_model(ev: &ChannelMessageEvent) -> String {
    let value = serde_json::json!({
        "type": "channel_message",
        "schema_version": ev.schema_version,
        "id": ev.id,
        "server": ev.server,
        "source": ev.source,
        "sender": ev.sender,
        "text": ev.text,
        "attachments": ev.attachments,
        "metadata": ev.metadata,
    });
    let json = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    format!("Inbound channel message:\n```json\n{json}\n```")
}

pub(crate) fn format_channel_message_for_display(ev: &ChannelMessageEvent) -> String {
    let source = ev.source.as_deref().unwrap_or(ev.server.as_str());
    let mut text = match ev.sender.as_deref() {
        Some(sender) if !sender.is_empty() => {
            format!("Inbound {source} message from {sender}:\n{}", ev.text)
        }
        _ => format!("Inbound {source} message:\n{}", ev.text),
    };

    if !ev.attachments.is_empty() {
        text.push_str("\n\nAttachments:");
        for attachment in &ev.attachments {
            let label = attachment
                .file_name
                .as_deref()
                .or_else(|| {
                    attachment
                        .path
                        .as_ref()
                        .and_then(|path| path.file_name()?.to_str())
                })
                .unwrap_or(attachment.kind.as_str());
            match &attachment.path {
                Some(path) => text.push_str(&format!("\n- {label}: {}", path.display())),
                None => text.push_str(&format!("\n- {label}")),
            }
        }
    }
    text
}

pub(crate) fn channel_message_local_images(ev: &ChannelMessageEvent) -> Vec<LocalImageAttachment> {
    ev.attachments
        .iter()
        .filter(|attachment| is_channel_image_attachment(attachment))
        .filter_map(|attachment| attachment.path.clone())
        .enumerate()
        .map(|(idx, path)| LocalImageAttachment {
            placeholder: local_image_label_text(idx + 1),
            path,
        })
        .collect()
}

fn is_channel_image_attachment(attachment: &ChannelMessageAttachment) -> bool {
    attachment
        .mime_type
        .as_deref()
        .is_some_and(|mime_type| mime_type.starts_with("image/"))
        || matches!(attachment.kind.as_str(), "photo" | "image")
}

impl ChatWidget {
    pub(crate) fn on_channel_message(&mut self, ev: ChannelMessageEvent) {
        let history_text = format_channel_message_for_display(&ev);
        let text = format_channel_message_for_model(&ev);
        let local_images = channel_message_local_images(&ev);
        let _ = self.submit_user_message_with_history_and_shell_escape_policy(
            UserMessage {
                text,
                local_images,
                remote_image_urls: Vec::new(),
                text_elements: Vec::new(),
                mention_bindings: Vec::new(),
            },
            UserMessageHistoryRecord::Override(UserMessageHistoryOverride {
                text: history_text,
                text_elements: Vec::new(),
            }),
            ShellEscapePolicy::Disallow,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::protocol::ChannelMessageAttachment;
    use codex_protocol::protocol::ChannelMessageEvent;
    use insta::assert_snapshot;
    use std::path::PathBuf;

    fn event() -> ChannelMessageEvent {
        ChannelMessageEvent {
            id: "telegram:123:456".to_string(),
            schema_version: 1,
            server: "telegram-channel".to_string(),
            source: Some("telegram".to_string()),
            sender: Some("123".to_string()),
            text: "ping".to_string(),
            attachments: Vec::new(),
            metadata: Some(serde_json::json!({ "chat_id": "123" })),
        }
    }

    #[test]
    fn model_payload_keeps_routing_metadata() {
        let model_text = format_channel_message_for_model(&event());

        assert!(model_text.contains("\"id\": \"telegram:123:456\""));
        assert!(model_text.contains("\"chat_id\": \"123\""));
        assert!(model_text.contains("\"text\": \"ping\""));
    }

    #[test]
    fn display_hides_raw_routing_json() {
        let display_text = format_channel_message_for_display(&event());

        assert_snapshot!("channel_message_display", display_text);
        assert!(!display_text.contains("```json"));
        assert!(!display_text.contains("chat_id"));
    }

    #[test]
    fn image_attachment_becomes_local_image() {
        let mut event = event();
        event.text = "photo".to_string();
        event.attachments = vec![ChannelMessageAttachment {
            kind: "photo".to_string(),
            path: Some(PathBuf::from("/tmp/photo.jpg")),
            mime_type: Some("image/jpeg".to_string()),
            file_name: Some("photo.jpg".to_string()),
            file_size: Some(123),
            metadata: None,
        }];

        let display_text = format_channel_message_for_display(&event);
        let local_images = channel_message_local_images(&event);

        assert!(display_text.contains("Attachments:"));
        assert!(display_text.contains("/tmp/photo.jpg"));
        assert_eq!(local_images.len(), 1);
        assert_eq!(local_images[0].path, PathBuf::from("/tmp/photo.jpg"));
    }
}
