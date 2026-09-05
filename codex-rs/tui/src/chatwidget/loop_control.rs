//! Local autonomous loop control for the TUI.

use super::*;
use codex_context_fragments::ContextualUserFragment;
use codex_context_fragments::LoopContinuation;

const LOOP_USAGE: &str = "Usage: /loop <minutes> [--max N] <prompt> | /loop now [--max N] <prompt> | /loop --immediate [--max N] <prompt> | /loop once <prompt> | /loop stop | /loop status";

impl ChatWidget {
    pub(crate) fn handle_loop_tool_call(
        &mut self,
        thread_id: &str,
        arguments: serde_json::Value,
    ) -> codex_app_server_protocol::DynamicToolCallResponse {
        use codex_app_server_protocol::DynamicToolCallOutputContentItem;
        use codex_app_server_protocol::DynamicToolCallResponse;
        if self.thread_id.is_none_or(|id| id.to_string() != thread_id) {
            return crate::dynamic_tools::failure_response(
                "Local loop control requires this TUI's active thread.",
            );
        }
        let Ok(event) = serde_json::from_value::<LoopControlEvent>(arguments) else {
            return crate::dynamic_tools::failure_response("Invalid local loop control arguments.");
        };
        if event.action == LoopControlAction::Start && self.loop_ui.model_start_blocked {
            return crate::dynamic_tools::failure_response(
                "Loop was stopped; a late model request cannot restart it during the same turn.",
            );
        }
        let reason = event.reason.clone();
        let starting = event.action == LoopControlAction::Start;
        let generation = self.loop_ui.generation;
        self.on_loop_control(event);
        let success = !starting || generation != self.loop_ui.generation;
        let Ok(text) = serde_json::to_string(&self.loop_ui.snapshot(reason)) else {
            return crate::dynamic_tools::failure_response(
                "Could not serialize local loop status.",
            );
        };
        DynamicToolCallResponse {
            content_items: vec![DynamicToolCallOutputContentItem::InputText { text }],
            success,
        }
    }

    pub(super) fn add_loop_status_output(&mut self) {
        self.add_info_message(self.loop_ui.status_summary(), Some(LOOP_USAGE.to_string()));
    }

    pub(crate) fn on_loop_control(&mut self, event: LoopControlEvent) {
        if event
            .reason
            .as_ref()
            .is_some_and(|reason| reason.len() > 512)
        {
            self.add_error_message("Loop reason must be at most 512 bytes.".to_string());
            return;
        }
        match event.action {
            LoopControlAction::Status => self.add_loop_status_output(),
            LoopControlAction::Stop => {
                self.stop_loop();
                let mut message = "Loop stopped by loop_control.".to_string();
                if let Some(reason) = event.reason.as_deref().map(str::trim)
                    && !reason.is_empty()
                {
                    message.push_str(" Reason: ");
                    message.push_str(reason);
                }

                self.add_info_message(message, /*hint*/ None);
            }
            LoopControlAction::Start => {
                if !self.loop_ui.model_start_blocked {
                    self.start_loop_from_control(event);
                }
            }
        }
    }

    fn start_loop_from_control(&mut self, event: LoopControlEvent) {
        if event.max_iterations == Some(0) {
            self.add_error_message("Loop max must be greater than zero.".to_string());
            return;
        }

        let mode = event.mode.unwrap_or(LoopControlMode::Immediate);
        let prompt = event.prompt.as_deref().unwrap_or_default();
        match mode {
            LoopControlMode::Timed => {
                let Some(interval_minutes) = event.interval_minutes else {
                    self.add_error_message(
                        "loop_control start mode=timed requires interval_minutes.".to_string(),
                    );
                    return;
                };
                if !(1..=525_600).contains(&interval_minutes) {
                    self.add_error_message(
                        "Loop interval must be greater than zero minutes.".to_string(),
                    );
                    return;
                }
                self.start_loop(
                    Some(interval_minutes),
                    /* immediate */ false,
                    event.max_iterations,
                    prompt,
                );
            }
            LoopControlMode::Immediate => {
                self.start_loop(
                    None,
                    /* immediate */ true,
                    event.max_iterations,
                    prompt,
                );
            }
            LoopControlMode::Once => {
                self.start_loop(None, /* immediate */ false, Some(1), prompt);
            }
        }
    }

    pub(super) fn handle_loop_command_args(&mut self, args: &str) {
        let trimmed = args.trim();
        let mut parts = trimmed.split_whitespace();
        let command = parts.next().unwrap_or("status").to_ascii_lowercase();
        match command.as_str() {
            "" | "status" => self.add_loop_status_output(),
            "stop" | "pause" => {
                self.stop_loop();
                self.add_loop_status_output();
            }
            "once" => {
                let rest = trimmed[command.len()..].trim();
                self.start_one_shot_loop(rest);
            }
            "now" | "immediate" | "--immediate" | "--now" => {
                let rest = trimmed[command.len()..].trim();
                self.start_immediate_loop_from_args(rest);
            }
            _ => {
                if command.parse::<u64>().is_ok() {
                    self.start_timed_loop_from_args(trimmed);
                } else {
                    self.add_error_message(LOOP_USAGE.to_string());
                }
            }
        }
    }

    fn start_timed_loop_from_args(&mut self, args: &str) {
        let mut parts = args.splitn(2, char::is_whitespace);
        let Some(minutes_text) = parts.next().filter(|text| !text.is_empty()) else {
            self.add_error_message("Usage: /loop <minutes> <prompt>".to_string());
            return;
        };
        let Ok(minutes) = minutes_text.parse::<u64>() else {
            self.add_error_message(
                "Loop interval must be a positive number of minutes.".to_string(),
            );
            return;
        };
        if !(1..=525_600).contains(&minutes) {
            self.add_error_message("Loop interval must be greater than zero minutes.".to_string());
            return;
        }
        let rest = parts.next().map(str::trim).unwrap_or_default();
        let Some((max_iterations, prompt)) = self.parse_loop_options(rest) else {
            return;
        };
        self.start_loop(
            Some(minutes),
            /* immediate */ false,
            max_iterations,
            &prompt,
        );
    }

    fn start_immediate_loop_from_args(&mut self, args: &str) {
        let Some((max_iterations, prompt)) = self.parse_loop_options(args) else {
            return;
        };
        self.start_loop(None, /* immediate */ true, max_iterations, &prompt);
    }

    fn start_one_shot_loop(&mut self, prompt: &str) {
        self.start_loop(None, /* immediate */ false, Some(1), prompt);
    }

    fn parse_loop_options(&mut self, args: &str) -> Option<(Option<usize>, String)> {
        let mut tokens = args.split_whitespace().peekable();
        let mut max_iterations = None;
        let mut prompt_parts: Vec<&str> = Vec::new();

        while let Some(token) = tokens.next() {
            if token == "--max" {
                let Some(value) = tokens.next() else {
                    self.add_error_message(LOOP_USAGE.to_string());
                    return None;
                };
                let Ok(parsed) = value.parse::<usize>() else {
                    self.add_error_message("Loop max must be a positive number.".to_string());
                    return None;
                };
                if parsed == 0 {
                    self.add_error_message("Loop max must be greater than zero.".to_string());
                    return None;
                }
                max_iterations = Some(parsed);
            } else {
                prompt_parts.push(token);
                prompt_parts.extend(tokens);
                break;
            }
        }

        Some((max_iterations, prompt_parts.join(" ")))
    }

    fn start_loop(
        &mut self,
        interval_minutes: Option<u64>,
        immediate: bool,
        max_iterations: Option<usize>,
        prompt: &str,
    ) {
        if !self.is_session_configured() {
            self.add_error_message("Start a session before enabling a loop.".to_string());
            return;
        }
        if prompt.len() > 1024 {
            self.add_error_message("Loop prompt must be at most 1024 bytes.".to_string());
            return;
        }
        self.loop_ui.model_start_blocked = false;
        self.loop_ui.thread_id = self.thread_id;
        self.loop_ui.enabled = true;
        self.loop_ui.completed_iterations = 0;
        self.loop_ui.interval_minutes = interval_minutes;
        self.loop_ui.immediate = immediate;
        self.loop_ui.max_iterations = max_iterations;
        self.loop_ui.timer_pending = false;
        self.loop_ui.generation = self.loop_ui.generation.wrapping_add(1);
        self.loop_ui.prompt = prompt.trim().to_string();

        self.add_loop_status_output();
        if !self.is_user_turn_pending_or_running() && !self.has_queued_follow_up_messages() {
            self.maybe_submit_loop_follow_up();
        }
    }

    pub(super) fn stop_loop(&mut self) {
        self.loop_ui.model_start_blocked = true;
        self.loop_ui.enabled = false;
        self.loop_ui.timer_pending = false;
        self.loop_ui.generation = self.loop_ui.generation.wrapping_add(1);
    }

    fn stop_loop_after_max_iterations(&mut self) -> bool {
        let Some(max_iterations) = self.loop_ui.max_iterations else {
            return false;
        };
        if self.loop_ui.completed_iterations < max_iterations {
            return false;
        }
        self.stop_loop();
        self.add_info_message(
            format!("Loop stopped after {max_iterations} iteration(s)."),
            /* hint */ None,
        );
        true
    }

    pub(super) fn resume_loop_after_goal(&mut self) {
        if !self.is_user_turn_pending_or_running()
            && !self.thread_usage.replaying_turn_completion
            && !self.has_queued_follow_up_messages()
            && self.input_queue.pending_steers.is_empty()
        {
            self.maybe_continue_loop_after_turn();
        }
    }

    pub(super) fn maybe_continue_loop_after_turn(&mut self) -> bool {
        if !self.loop_ui.enabled {
            return false;
        }
        if self.loop_ui.thread_id != self.thread_id {
            self.stop_loop();
            return false;
        }
        if self.stop_loop_after_max_iterations() {
            return false;
        }
        if self.loop_ui.immediate || self.loop_ui.interval_minutes.is_none() {
            return self.maybe_submit_loop_follow_up();
        }
        self.schedule_loop_timer()
    }

    fn schedule_loop_timer(&mut self) -> bool {
        let Some(interval_minutes) = self.loop_ui.interval_minutes else {
            return false;
        };
        if !self.loop_ui.enabled || self.loop_ui.timer_pending {
            return false;
        }
        if self.stop_loop_after_max_iterations() {
            return false;
        }
        self.loop_ui.timer_pending = true;

        let generation = self.loop_ui.generation;
        let owner = self.loop_ui.owner;
        let thread_id = self.thread_id;
        let tx = self.app_event_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(interval_minutes.saturating_mul(60))).await;
            tx.send(AppEvent::LoopTimerFired {
                generation,
                owner,
                thread_id,
            });
        });
        self.add_info_message(
            format!("Loop scheduled next iteration in {interval_minutes} minute(s)."),
            /* hint */ None,
        );
        true
    }

    pub(crate) fn on_loop_timer_fired(
        &mut self,
        owner: uuid::Uuid,
        thread_id: Option<ThreadId>,
        generation: u64,
    ) {
        if owner != self.loop_ui.owner
            || thread_id != self.thread_id
            || generation != self.loop_ui.generation
        {
            return;
        }
        self.loop_ui.timer_pending = false;

        if !self.maybe_submit_loop_follow_up() && self.loop_ui.enabled {
            self.schedule_loop_timer();
        }
    }

    fn maybe_submit_loop_follow_up(&mut self) -> bool {
        if !self.loop_ui.enabled
            || self.loop_ui.thread_id != self.thread_id
            || self.has_queued_follow_up_messages()
            || !self.input_queue.pending_steers.is_empty()
            || self.input_queue.rate_limit_recovery_pending
            || self
                .current_goal_status
                .as_ref()
                .is_some_and(GoalStatusState::is_active)
            || !self.is_session_configured()
            || self.is_user_turn_pending_or_running()
        {
            return false;
        }
        if self.stop_loop_after_max_iterations() {
            return false;
        }

        let iteration = self.loop_ui.completed_iterations.saturating_add(1);
        let prompt = self.loop_ui.active_prompt();
        let submitted = self
            .submit_user_message_with_history_and_shell_escape_policy(
                UserMessage {
                    text: prompt,
                    local_images: Vec::new(),
                    remote_image_urls: Vec::new(),
                    text_elements: Vec::new(),
                    mention_bindings: Vec::new(),
                },
                UserMessageHistoryRecord::Override(UserMessageHistoryOverride {
                    text: format!("/loop iteration {iteration}"),
                    text_elements: Vec::new(),
                }),
                ShellEscapePolicy::Disallow,
            )
            .0;
        if !submitted {
            self.stop_loop();
            return false;
        }
        self.loop_ui.completed_iterations = iteration;
        self.add_info_message(
            format!("Loop iteration {iteration} submitted."),
            /*hint*/ None,
        );
        if self.loop_ui.interval_minutes.is_none() && !self.loop_ui.immediate {
            self.stop_loop();
        }
        true
    }
}

pub(super) struct LoopUiState {
    pub(super) enabled: bool,
    pub(super) model_start_blocked: bool,
    pub(super) completed_iterations: usize,
    pub(super) interval_minutes: Option<u64>,
    pub(super) immediate: bool,
    pub(super) max_iterations: Option<usize>,
    pub(super) timer_pending: bool,
    pub(super) generation: u64,
    pub(super) owner: uuid::Uuid,
    pub(super) thread_id: Option<ThreadId>,
    pub(super) prompt: String,
}

impl Default for LoopUiState {
    fn default() -> Self {
        Self {
            enabled: false,
            model_start_blocked: false,
            completed_iterations: 0,
            interval_minutes: None,
            immediate: false,
            max_iterations: None,
            timer_pending: false,
            generation: 0,
            owner: uuid::Uuid::new_v4(),
            thread_id: None,
            prompt: String::new(),
        }
    }
}

impl LoopUiState {
    fn active_prompt(&self) -> String {
        if self.prompt.trim().is_empty() {
            LoopContinuation.render()
        } else {
            self.prompt.clone()
        }
    }

    fn status_summary(&self) -> String {
        self.snapshot(None).summary()
    }

    fn snapshot(&self, last_reason: Option<String>) -> LoopStatusSnapshot {
        LoopStatusSnapshot {
            active: self.enabled,
            mode: if self.immediate {
                LoopStatusMode::Immediate
            } else if self.interval_minutes.is_some() {
                LoopStatusMode::Timed
            } else {
                LoopStatusMode::Once
            },
            completed_iterations: self.completed_iterations,
            interval_minutes: self.interval_minutes,
            max_iterations: self.max_iterations,
            timer_pending: self.timer_pending,
            prompt: self.prompt.clone(),
            last_reason,
        }
    }
}
