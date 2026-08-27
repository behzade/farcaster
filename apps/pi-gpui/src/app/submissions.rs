//! Composer submission lifecycle, including image attachment ownership.

use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{ClipboardItem, Context, Window};

use super::{ComposerImage, PiApp, slash_commands};
use crate::{
    app::slash_commands::{BuiltinInvocation, BuiltinSlashCommand},
    composer_sessions::{ComposerSnapshot, session_target},
    conversation::TranscriptKind,
    protocol::{PromptImage, PromptMode},
    runtime::RuntimeCommand,
    sessions::{SessionSummary, normalize_session_path},
    user_invocations,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PendingSubmission {
    pub(super) text: String,
    pub(super) images: Vec<ComposerImage>,
    pub(super) result: Option<(bool, Option<std::path::PathBuf>)>,
}

impl PiApp {
    pub(crate) fn submit(
        &mut self,
        value: String,
        mode: PromptMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_submit() {
            return;
        }
        if let Some(invocation) = slash_commands::builtin_invocation(&value) {
            self.submit_builtin(&value, invocation, window, cx);
            return;
        }
        self.capture_composer_session(cx);
        let target = self.composer_sessions.current_target().to_owned();
        let editor_text = self.composer.read(cx).value().to_string();
        let allow_while_running =
            slash_commands::is_immediate_extension(&value, &self.snapshot.commands);
        let mode = if allow_while_running {
            PromptMode::Normal
        } else {
            mode
        };
        let show_in_transcript = !self.snapshot.conversation.running;
        let images = self
            .composer_images
            .get(&target)
            .into_iter()
            .flatten()
            .map(|image| image.prompt.clone())
            .collect::<Vec<PromptImage>>();
        let inactive_session = inactive_session_for_target(
            &target,
            self.snapshot.selected_session.as_deref(),
            &self.sessions,
        );
        match self.runtime.send(RuntimeCommand::Prompt {
            target: target.clone(),
            mode,
            message: value.clone(),
            images,
            allow_while_running,
        }) {
            Ok(()) => {
                if let Some(path) = inactive_session {
                    self.set_session_active(path, cx);
                }
                self.begin_draft_submission(&target, &value);
                self.notify_session_rail(cx);
                self.composer_sessions.record_submission(&target, &value);
                let pending_images = self.composer_images.remove(&target).unwrap_or_default();
                let image_count = pending_images.len();
                self.pending_submissions.insert(
                    target.clone(),
                    PendingSubmission {
                        text: editor_text.clone(),
                        images: pending_images,
                        result: None,
                    },
                );
                if self
                    .composer_sessions
                    .clear_submitted_text(&target, &editor_text)
                    && self.composer_sessions.current_target() == target
                {
                    self.apply_composer_snapshot(ComposerSnapshot::default(), window, cx);
                }
                let invocation =
                    user_invocations::contains_invocation(&value, &self.snapshot.commands);
                let snapshot = Arc::make_mut(&mut self.snapshot);
                let index = snapshot.conversation.items.len();
                let conversation = Arc::make_mut(&mut snapshot.conversation);
                if show_in_transcript {
                    conversation.push_local_user(value, image_count, invocation);
                }
                conversation.running = true;
                snapshot.status = "Working".into();
                if show_in_transcript {
                    self.mark_transcript_changed(index, index == 0);
                }
                self.jump_to_latest(cx);
                cx.notify();
            }
            Err(error) => {
                let snapshot = Arc::make_mut(&mut self.snapshot);
                let index = snapshot.conversation.items.len();
                Arc::make_mut(&mut snapshot.conversation).push_transport_error(error);
                self.mark_transcript_changed(index, index == 0);
                cx.notify();
            }
        }
    }

    fn submit_builtin(
        &mut self,
        value: &str,
        invocation: BuiltinInvocation<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(invocation.command, BuiltinSlashCommand::Reload) && self.session_is_busy() {
            self.push_builtin_error(
                "Reload not started",
                "Wait for the current response to finish before reloading.",
                cx,
            );
            return;
        }

        self.consume_composer_command(value, window, cx);
        match invocation.command {
            BuiltinSlashCommand::Model => {
                if let Some(reference) = invocation.arguments {
                    let model = self.snapshot.models.iter().find(|model| {
                        format!("{}/{}", model.provider, model.id) == reference
                            || model.id == reference
                    });
                    if let Some(model) = model.cloned() {
                        self.select_model(&model, cx);
                    } else {
                        self.push_builtin_error(
                            "Model not changed",
                            &format!("No available model exactly matches {reference}."),
                            cx,
                        );
                    }
                } else {
                    Arc::make_mut(&mut self.snapshot).status = "Choose a model below".into();
                    self.composer_focus.focus(window, cx);
                    cx.notify();
                }
            }
            BuiltinSlashCommand::Export => {
                if invocation
                    .arguments
                    .is_some_and(|path| path.ends_with(".jsonl"))
                {
                    self.push_builtin_error(
                        "Export unavailable",
                        "Pi’s public RPC supports HTML export, not JSONL export.",
                        cx,
                    );
                } else {
                    self.send(RuntimeCommand::ExportHtml {
                        output_path: invocation.arguments.map(str::to_owned),
                    });
                }
            }
            BuiltinSlashCommand::Copy => {
                let mut parts = self
                    .snapshot
                    .conversation
                    .items
                    .iter_rev()
                    .take_while(|item| item.kind != TranscriptKind::User)
                    .filter(|item| item.kind == TranscriptKind::Assistant && !item.text.is_empty())
                    .map(|item| item.text.clone())
                    .collect::<Vec<_>>();
                parts.reverse();
                let text = parts.join("\n\n");
                if !text.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    Arc::make_mut(&mut self.snapshot).status = "Copied last response".into();
                    cx.notify();
                } else {
                    self.push_builtin_error(
                        "Nothing copied",
                        "This session has no assistant response.",
                        cx,
                    );
                }
            }
            BuiltinSlashCommand::Name => {
                if let Some(name) = invocation.arguments {
                    self.send(RuntimeCommand::SetSessionName(name.to_owned()));
                } else {
                    self.push_builtin_error(
                        "Name not changed",
                        "Use /name <session name> in GPUI.",
                        cx,
                    );
                }
            }
            BuiltinSlashCommand::Session => self.open_run_sheet(window, cx),
            BuiltinSlashCommand::New => self.open_picker(
                super::PickerScope::Projects(super::ProjectPickerIntent::NewSession),
                window,
                cx,
            ),
            BuiltinSlashCommand::Compact => self.send(RuntimeCommand::Compact {
                custom_instructions: invocation.arguments.map(str::to_owned),
            }),
            BuiltinSlashCommand::Resume => self.open_sessions_sheet(window, cx),
            BuiltinSlashCommand::Reload => self.send(RuntimeCommand::Reload),
            BuiltinSlashCommand::Trust => self.open_project_trust(window, cx),
            BuiltinSlashCommand::Login => self.send(RuntimeCommand::Login(
                invocation.arguments.map(str::to_owned),
            )),
            BuiltinSlashCommand::Quit => cx.quit(),
            BuiltinSlashCommand::Settings
            | BuiltinSlashCommand::ScopedModels
            | BuiltinSlashCommand::Import
            | BuiltinSlashCommand::Share
            | BuiltinSlashCommand::Changelog
            | BuiltinSlashCommand::Hotkeys
            | BuiltinSlashCommand::Fork
            | BuiltinSlashCommand::Clone
            | BuiltinSlashCommand::Tree
            | BuiltinSlashCommand::Logout => self.push_builtin_error(
                "Command unavailable",
                &format!(
                    "/{} is not available in GPUI and was not sent as a prompt.",
                    invocation.name
                ),
                cx,
            ),
        }
    }

    fn session_is_busy(&self) -> bool {
        self.snapshot.conversation.running
            || self.snapshot.conversation.compacting
            || matches!(
                self.snapshot.live_status.as_str(),
                "Working" | "Compacting" | "Retrying"
            )
    }

    fn consume_composer_command(
        &mut self,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_composer_session(cx);
        let target = self.composer_sessions.current_target().to_owned();
        let editor_text = self.composer.read(cx).value().to_string();
        self.composer_sessions.record_submission(&target, value);
        if self
            .composer_sessions
            .clear_submitted_text(&target, &editor_text)
            && self.composer_sessions.current_target() == target
        {
            self.apply_composer_snapshot(ComposerSnapshot::default(), window, cx);
        }
    }

    fn push_builtin_error(&mut self, label: &str, message: &str, cx: &mut Context<Self>) {
        let snapshot = Arc::make_mut(&mut self.snapshot);
        let index = snapshot.conversation.items.len();
        Arc::make_mut(&mut snapshot.conversation).push_local_error(label, message.to_owned());
        self.mark_transcript_changed(index, index == 0);
        cx.notify();
    }

    pub(crate) fn can_submit(&self) -> bool {
        can_submit_to(
            &self.pending_submissions,
            self.composer_sessions.current_target(),
        )
    }

    pub(crate) fn handle_composer_escape(&mut self) {
        let (abort, arm) = composer_escape(
            self.snapshot.conversation.running,
            !self.snapshot.conversation.queue.steering.is_empty(),
            self.composer_sessions.current_target(),
            self.composer_escape_armed.as_ref(),
            Instant::now(),
        );
        self.composer_escape_armed = arm;
        if abort {
            self.send(RuntimeCommand::Abort);
        }
    }

    pub(crate) fn enter_mode(&self) -> PromptMode {
        prompt_mode_for_enter(self.snapshot.conversation.running)
    }

    pub(crate) fn submit_follow_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.composer.read(cx).value().trim().to_owned();
        if !value.is_empty() || self.has_composer_images() {
            let mode = prompt_mode_for_follow_up(self.snapshot.conversation.running);
            self.submit(value, mode, window, cx);
        }
    }

    pub(super) fn resolve_pending_submission(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let completed = self
            .pending_submissions
            .iter()
            .filter_map(|(target, pending)| {
                pending
                    .result
                    .clone()
                    .map(|(accepted, session)| (target.clone(), accepted, session))
            })
            .collect::<Vec<_>>();
        for (target, accepted, session) in completed {
            let Some(pending) = self.pending_submissions.remove(&target) else {
                continue;
            };
            if accepted {
                continue;
            }

            let restored = self
                .composer_sessions
                .restore_submitted_text(&target, pending.text.clone());
            if !pending.images.is_empty() {
                self.composer_images
                    .entry(target.clone())
                    .or_insert_with(|| pending.images.clone());
            }
            if let Some(snapshot) = restored
                && self.composer_sessions.current_target() == target
            {
                self.apply_composer_snapshot(snapshot, window, cx);
            }
            if let Some(session_key) = rejected_attachment_target(
                &pending.text,
                !pending.images.is_empty(),
                &target,
                self.composer_sessions.current_target(),
                session.as_deref(),
            ) {
                self.composer_sessions.promote(&target, session_key.clone());
                self.promote_composer_images(&target, &session_key);
            }
        }
    }
}

fn can_submit_to(
    pending: &std::collections::HashMap<String, PendingSubmission>,
    target: &str,
) -> bool {
    !pending.contains_key(target)
}

fn inactive_session_for_target(
    target: &str,
    selected_session: Option<&Path>,
    sessions: &[SessionSummary],
) -> Option<std::path::PathBuf> {
    let selected = selected_session?;
    (target == session_target(selected))
        .then(|| {
            sessions
                .iter()
                .find(|session| session.path == selected && (session.in_review || session.archived))
                .map(|session| session.path.clone())
        })
        .flatten()
}

fn rejected_attachment_target(
    text: &str,
    has_images: bool,
    pending_target: &str,
    current_target: &str,
    session: Option<&Path>,
) -> Option<String> {
    (text.trim().is_empty() && has_images && current_target != pending_target)
        .then(|| session.map(normalize_session_path))
        .flatten()
        .map(|path| session_target(&path))
}

fn prompt_mode_for_enter(running: bool) -> PromptMode {
    if running {
        PromptMode::Steer
    } else {
        PromptMode::Normal
    }
}

fn prompt_mode_for_follow_up(running: bool) -> PromptMode {
    if running {
        PromptMode::FollowUp
    } else {
        PromptMode::Normal
    }
}

const COMPOSER_ABORT_DOUBLE_TAP: Duration = Duration::from_millis(500);

fn composer_escape(
    running: bool,
    has_queued_steer: bool,
    current_target: &str,
    armed: Option<&(String, Instant)>,
    now: Instant,
) -> (bool, Option<(String, Instant)>) {
    if !running {
        return (false, None);
    }
    let armed_here = armed.is_some_and(|(target, at)| {
        target == current_target && now.saturating_duration_since(*at) <= COMPOSER_ABORT_DOUBLE_TAP
    });
    if has_queued_steer || armed_here {
        (true, None)
    } else {
        (false, Some((current_target.to_owned(), now)))
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant, SystemTime};

    use super::*;
    use crate::sessions::UsageSummary;

    fn session(path: &str, archived: bool) -> SessionSummary {
        SessionSummary::from_cached(
            "test".into(),
            path.into(),
            "/project".into(),
            "Test".into(),
            String::new(),
            String::new(),
            None,
            SystemTime::UNIX_EPOCH,
            0,
            UsageSummary::default(),
            archived,
            false,
            String::new(),
        )
    }

    fn pending() -> PendingSubmission {
        PendingSubmission {
            text: "submitted".into(),
            images: Vec::new(),
            result: None,
        }
    }

    #[test]
    fn review_and_archived_sessions_activate_when_their_message_is_sent() {
        let path = Path::new("/sessions/inactive.jsonl");
        let archived = [session("/sessions/inactive.jsonl", true)];
        assert_eq!(
            inactive_session_for_target(&session_target(path), Some(path), &archived),
            Some(path.to_path_buf())
        );
        let review = [session("/sessions/inactive.jsonl", false).with_review(true)];
        assert_eq!(
            inactive_session_for_target(&session_target(path), Some(path), &review),
            Some(path.to_path_buf())
        );
        assert_eq!(
            inactive_session_for_target("session:/sessions/other.jsonl", Some(path), &review),
            None
        );
        let active = [session("/sessions/inactive.jsonl", false)];
        assert_eq!(
            inactive_session_for_target(&session_target(path), Some(path), &active),
            None
        );
    }

    #[test]
    fn rejected_attachment_only_submission_moves_to_its_real_session_after_navigation() {
        let session = Path::new("/sessions/one.jsonl");
        assert_eq!(
            rejected_attachment_target("", true, "draft:one", "session:other", Some(session),),
            Some(session_target(session))
        );
        assert_eq!(
            rejected_attachment_target("typed", true, "draft:one", "session:other", Some(session),),
            None
        );
        assert_eq!(
            rejected_attachment_target("", true, "draft:one", "draft:one", Some(session)),
            None
        );
    }

    #[test]
    fn pending_submission_only_blocks_its_own_composer() {
        let pending = std::collections::HashMap::from([("session:compacting".into(), pending())]);

        assert!(!can_submit_to(&pending, "session:compacting"));
        assert!(can_submit_to(&pending, "session:other"));
        assert!(can_submit_to(&pending, "draft:new"));
    }

    #[test]
    fn enter_prompts_when_idle_and_steers_while_running() {
        assert_eq!(prompt_mode_for_enter(false), PromptMode::Normal);
        assert_eq!(prompt_mode_for_enter(true), PromptMode::Steer);
    }

    #[test]
    fn tab_prompts_when_idle_and_queues_a_follow_up_while_running() {
        assert_eq!(prompt_mode_for_follow_up(false), PromptMode::Normal);
        assert_eq!(prompt_mode_for_follow_up(true), PromptMode::FollowUp);
    }

    #[test]
    fn composer_escape_flushes_steer_or_double_taps_to_abort() {
        let t0 = Instant::now();
        let within = t0 + Duration::from_millis(400);
        let expired = t0 + Duration::from_millis(501);
        let one = "session:one";
        let two = "session:two";
        let armed = (one.to_owned(), t0);

        assert_eq!(composer_escape(false, true, one, None, t0), (false, None));
        assert_eq!(
            composer_escape(true, true, one, Some(&armed), t0),
            (true, None)
        );
        assert_eq!(
            composer_escape(true, false, one, None, t0),
            (false, Some((one.into(), t0)))
        );
        assert_eq!(
            composer_escape(true, false, one, Some(&armed), within),
            (true, None)
        );
        assert_eq!(
            composer_escape(true, false, one, Some(&armed), expired),
            (false, Some((one.into(), expired)))
        );
        assert_eq!(
            composer_escape(true, false, two, Some(&armed), t0),
            (false, Some((two.into(), t0)))
        );
    }
}
