//! Composer submission lifecycle, including image attachment ownership.

use std::{path::Path, sync::Arc};

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
    pub(super) target: String,
    pub(super) text: String,
    pub(super) images: Vec<ComposerImage>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SubmissionResolution {
    Accepted,
    Rejected,
    Ignore,
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
        let archived_session = archived_session_for_target(
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
                if let Some(path) = archived_session {
                    self.set_session_settled(path, false, cx);
                }
                self.begin_draft_submission(&target, &value);
                self.notify_session_rail(cx);
                self.composer_sessions.record_submission(&target, &value);
                let pending_images = self.composer_images.remove(&target).unwrap_or_default();
                let image_count = pending_images.len();
                self.pending_submission = Some(PendingSubmission {
                    target: target.clone(),
                    text: editor_text.clone(),
                    images: pending_images,
                });
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
                    .iter()
                    .rev()
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
        self.pending_submission.is_none()
    }

    pub(crate) fn enter_mode(&self) -> PromptMode {
        prompt_mode_for_enter(self.snapshot.conversation.running)
    }

    pub(crate) fn submit_follow_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.composer.read(cx).value().trim().to_owned();
        if !value.is_empty() || self.has_composer_images() {
            self.submit(value, PromptMode::FollowUp, window, cx);
        }
    }

    pub(super) fn resolve_pending_submission(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((target, accepted, session)) = self.pending_submission_result.take() else {
            return;
        };
        let resolution = submission_resolution(self.pending_submission.as_ref(), &target, accepted);
        if resolution == SubmissionResolution::Ignore {
            return;
        }
        let Some(pending) = self.pending_submission.take() else {
            return;
        };
        if resolution == SubmissionResolution::Accepted {
            return;
        }

        let restored = self
            .composer_sessions
            .restore_submitted_text(&pending.target, pending.text.clone());
        if !pending.images.is_empty() {
            self.composer_images
                .entry(pending.target.clone())
                .or_insert_with(|| pending.images.clone());
        }
        if let Some(snapshot) = restored
            && self.composer_sessions.current_target() == pending.target
        {
            self.apply_composer_snapshot(snapshot, window, cx);
        }
        if let Some(session_key) = rejected_attachment_target(
            &pending.text,
            !pending.images.is_empty(),
            &pending.target,
            self.composer_sessions.current_target(),
            session.as_deref(),
        ) {
            self.composer_sessions
                .promote(&pending.target, session_key.clone());
            self.promote_composer_images(&pending.target, &session_key);
        }
    }
}

fn archived_session_for_target(
    target: &str,
    selected_session: Option<&Path>,
    sessions: &[SessionSummary],
) -> Option<std::path::PathBuf> {
    let selected = selected_session?;
    (target == session_target(selected))
        .then(|| {
            sessions
                .iter()
                .find(|session| session.path == selected && session.settled)
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

fn submission_resolution(
    pending: Option<&PendingSubmission>,
    target: &str,
    accepted: bool,
) -> SubmissionResolution {
    let Some(_) = pending.filter(|pending| pending.target == target) else {
        return SubmissionResolution::Ignore;
    };
    if accepted {
        SubmissionResolution::Accepted
    } else {
        SubmissionResolution::Rejected
    }
}

fn prompt_mode_for_enter(running: bool) -> PromptMode {
    if running {
        PromptMode::Steer
    } else {
        PromptMode::Normal
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use super::*;
    use crate::sessions::UsageSummary;

    fn session(path: &str, settled: bool) -> SessionSummary {
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
            settled,
            false,
            String::new(),
        )
    }

    fn pending() -> PendingSubmission {
        PendingSubmission {
            target: "session:test".into(),
            text: "submitted".into(),
            images: Vec::new(),
        }
    }

    #[test]
    fn submission_results_distinguish_acceptance_and_rejection() {
        let pending = pending();
        assert_eq!(
            submission_resolution(Some(&pending), "session:test", true),
            SubmissionResolution::Accepted
        );
        assert_eq!(
            submission_resolution(Some(&pending), "session:test", false),
            SubmissionResolution::Rejected
        );
    }

    #[test]
    fn archived_selected_session_is_restored_when_its_message_is_sent() {
        let path = Path::new("/sessions/archived.jsonl");
        let sessions = [session("/sessions/archived.jsonl", true)];
        assert_eq!(
            archived_session_for_target(&session_target(path), Some(path), &sessions),
            Some(path.to_path_buf())
        );
        assert_eq!(
            archived_session_for_target("session:/sessions/other.jsonl", Some(path), &sessions),
            None
        );
        let active_sessions = [session("/sessions/archived.jsonl", false)];
        assert_eq!(
            archived_session_for_target(&session_target(path), Some(path), &active_sessions),
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
    fn stale_submission_result_is_ignored() {
        let pending = pending();
        assert_eq!(
            submission_resolution(Some(&pending), "session:other", false),
            SubmissionResolution::Ignore
        );
    }

    #[test]
    fn enter_prompts_when_idle_and_steers_while_running() {
        assert_eq!(prompt_mode_for_enter(false), PromptMode::Normal);
        assert_eq!(prompt_mode_for_enter(true), PromptMode::Steer);
    }
}
