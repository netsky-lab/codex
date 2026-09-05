use super::ContextualUserFragment;
use codex_protocol::models::ContentItemKind;

/// Default continuation requested by the user's local loop when no prompt was supplied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopContinuation;

impl ContextualUserFragment for LoopContinuation {
    fn role(&self) -> &'static str {
        "user"
    }

    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind("loop.continuation".to_string())
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("", "")
    }

    fn body(&self) -> String {
        "Continue working autonomously. Inspect the current project state, choose the next useful step toward the active goal, make progress, verify what you changed when possible, and report what you did. If there is no useful next step, explain that and wait for the user.".to_string()
    }
}
