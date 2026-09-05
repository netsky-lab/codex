use codex_protocol::models::ContentItemKind;
use codex_protocol::protocol::ChannelMessageEvent;

use super::ContextualUserFragment;

const MAX_CHANNEL_CONTEXT_BYTES: usize = 8 * 1024;

/// Bounded channel input retaining the sender's text and structured reply routing.
///
/// Construction rejects oversized envelopes instead of truncating routing identifiers
/// or JSON. The final textual fragment, including its wrapper, is at most 8 KiB.
/// This conservative byte bound permits ordinary 4096-character ASCII Telegram
/// messages; longer multibyte text or substantial quoted metadata can be rejected.
pub struct ChannelMessageContext {
    body: String,
}

impl ChannelMessageContext {
    /// Returns no fragment when the complete rendered envelope exceeds its byte budget.
    pub fn new(event: &ChannelMessageEvent) -> Option<Self> {
        let body = serde_json::json!({
            "type": "channel_message",
            "schema_version": event.schema_version,
            "id": event.id,
            "server": event.server,
            "source": event.source,
            "sender": event.sender,
            "text": event.text,
            "attachments": event.attachments,
            "metadata": event.metadata,
        })
        .to_string();
        let (start, end) = Self::type_markers();
        (body.len() <= MAX_CHANNEL_CONTEXT_BYTES - start.len() - end.len()).then_some(Self { body })
    }
}

impl ContextualUserFragment for ChannelMessageContext {
    fn role(&self) -> &'static str {
        "user"
    }

    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind("user.channel_message".to_string())
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("Inbound channel message:\n```json\n", "\n```")
    }

    fn body(&self) -> String {
        self.body.clone()
    }
}
