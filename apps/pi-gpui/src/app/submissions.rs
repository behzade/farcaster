//! Composer submission lifecycle, including image attachment ownership.

use std::sync::Arc;

use gpui::{ClipboardItem, Context, Window};

use super::{ComposerImage, PiApp, slash_commands};
use crate::{
    app::slash_commands::{BuiltinInvocation, BuiltinSlashCommand},
    composer_sessions::ComposerSnapshot,
    conversation::TranscriptKind,
    protocol::{PromptImage, PromptMode},
    runtime::RuntimeCommand,
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
    Rollback,
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
        let images = self
            .composer_images
            .get(&target)
            .into_iter()
            .flatten()
            .map(|image| image.prompt.clone())
            .collect::<Vec<PromptImage>>();
        match self.runtime.send(RuntimeCommand::Prompt {
            target: target.clone(),
            mode,
            message: value.clone(),
            images,
            allow_while_running,
        }) {
            Ok(()) => {
                self.composer_sessions.record_submission(&target, &value);
                let pending_images = self.composer_images.remove(&target).unwrap_or_default();
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
                self.jump_to_latest(cx);
            }
            Err(error) => {
                let snapshot = Arc::make_mut(&mut self.snapshot);
                let index = snapshot.conversation.items.len();
                snapshot.conversation.push_transport_error(error);
                self.transcript_list.splice(index..index, 1);
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
                    self.open_run_sheet(window, cx);
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
            BuiltinSlashCommand::New => self.new_session(self.project.clone(), window, cx),
            BuiltinSlashCommand::Compact => self.send(RuntimeCommand::Compact {
                custom_instructions: invocation.arguments.map(str::to_owned),
            }),
            BuiltinSlashCommand::Resume => self.open_sessions_sheet(window, cx),
            BuiltinSlashCommand::Reload => self.send(RuntimeCommand::Reload),
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
            | BuiltinSlashCommand::Trust
            | BuiltinSlashCommand::Login
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
        snapshot
            .conversation
            .push_local_error(label, message.to_owned());
        self.transcript_list.splice(index..index, 1);
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
        let Some((target, accepted)) = self.pending_submission_result.take() else {
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
            .prepend_failed_submission(&pending.target, &pending.text);
        if !pending.images.is_empty() {
            let new_images = self
                .composer_images
                .remove(&pending.target)
                .unwrap_or_default();
            let mut restored_images = pending.images;
            restored_images.extend(new_images);
            self.composer_images
                .insert(pending.target.clone(), restored_images);
        }
        if self.composer_sessions.current_target() == pending.target {
            self.apply_composer_snapshot(restored, window, cx);
        }
    }
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
        SubmissionResolution::Rollback
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
    use super::*;

    fn pending() -> PendingSubmission {
        PendingSubmission {
            target: "session:test".into(),
            text: "submitted".into(),
            images: Vec::new(),
        }
    }

    #[test]
    fn accepted_submission_stays_cleared_and_rejection_rolls_back() {
        let pending = pending();
        assert_eq!(
            submission_resolution(Some(&pending), "session:test", true),
            SubmissionResolution::Accepted
        );
        assert_eq!(
            submission_resolution(Some(&pending), "session:test", false),
            SubmissionResolution::Rollback
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
