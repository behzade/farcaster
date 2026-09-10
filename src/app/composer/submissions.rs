use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{Context, Window};

use super::{
    ComposerImage, ComposerPaste, FarcasterApp, pastes as composer_pastes, prompt_fragments,
};
use crate::{
    app::composer::sessions::{ComposerSnapshot, session_target},
    app::composer::user_invocations,
    protocol::{PromptImage, PromptMode},
    runtime::RuntimeCommand,
    sessions::{SessionSummary, normalize_session_path},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::app) struct PendingSubmission {
    // Runtime replies keep this target even after the draft is promoted.
    pub(in crate::app) submitted_target: String,
    pub(in crate::app) text: String,
    pub(in crate::app) images: Vec<ComposerImage>,
    pub(in crate::app) pastes: Vec<ComposerPaste>,
    pub(in crate::app) append_on_failure: bool,
    pub(in crate::app) result: Option<(bool, Option<std::path::PathBuf>)>,
}

impl FarcasterApp {
    pub(crate) fn submit(
        &mut self,
        value: String,
        mode: PromptMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_submit() || self.pending_project_trust_command.is_some() {
            return;
        }
        let backend = self.snapshot.harness.clone();
        let project = self.project.clone();
        if !self.ensure_backend_trust(&backend, &project, window, cx) {
            return;
        }
        self.capture_composer_session(cx);
        let target = self.composer_sessions.current_target().to_owned();
        let editor_text = self.composer.read(cx).value().to_string();
        let (mode, allow_while_running) =
            submission_delivery(&value, mode, &self.snapshot.commands);
        let show_in_transcript = !self.snapshot.conversation.running;
        let images = self
            .composer_images
            .get(&target)
            .into_iter()
            .flatten()
            .map(|image| image.prompt.clone())
            .collect::<Vec<PromptImage>>();
        let pastes = self
            .composer_pastes
            .get(&target)
            .cloned()
            .unwrap_or_default();
        let expansion = prompt_fragments::expand(&value);
        let resolved = expansion
            .as_ref()
            .map_or(value.as_str(), |expansion| expansion.message.as_str());
        let message = composer_pastes::append_pasted_files(resolved, &pastes);
        let display_message = expansion.as_ref().map(|expansion| {
            composer_pastes::append_pasted_file_links(&expansion.display, &pastes)
        });
        let invocation = expansion
            .as_ref()
            .map(|expansion| expansion.resolution.clone());
        let inactive_session = inactive_session_for_target(
            &target,
            self.snapshot.selected_session.as_deref(),
            &self.sessions,
        );
        match self.runtime.send(RuntimeCommand::Prompt {
            target: target.clone(),
            mode,
            message: message.clone(),
            display_message: display_message.clone(),
            invocation: invocation.clone(),
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
                let pending_pastes = self.composer_pastes.remove(&target).unwrap_or_default();
                let previews = Arc::new(
                    pending_images
                        .iter()
                        .map(|image| image.preview.clone())
                        .collect(),
                );
                self.pending_submissions.insert(
                    target.clone(),
                    PendingSubmission {
                        submitted_target: target.clone(),
                        text: editor_text.clone(),
                        images: pending_images,
                        pastes: pending_pastes,
                        append_on_failure: false,
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
                let snapshot = Arc::make_mut(&mut self.snapshot);
                let index = snapshot.conversation.items.len();
                let conversation = Arc::make_mut(&mut snapshot.conversation);
                if show_in_transcript {
                    match (display_message, invocation) {
                        (Some(display), Some(invocation)) => {
                            conversation.push_local_user_with_images(
                                display,
                                previews,
                                Some(invocation),
                            );
                        }
                        _ => {
                            let invocation =
                                user_invocations::contains_invocation(&value, &snapshot.commands);
                            conversation.push_local_user_with_images(
                                message,
                                previews,
                                invocation.then(String::new),
                            );
                        }
                    }
                }
                conversation.running = true;
                snapshot.status = "Working".into();
                if show_in_transcript {
                    self.mark_transcript_changed(index, index == 0, cx);
                }
                self.jump_to_latest(cx);
                cx.notify();
            }
            Err(error) => {
                let snapshot = Arc::make_mut(&mut self.snapshot);
                let index = snapshot.conversation.items.len();
                Arc::make_mut(&mut snapshot.conversation).push_transport_error(error);
                self.mark_transcript_changed(index, index == 0, cx);
                cx.notify();
            }
        }
    }

    pub(crate) fn can_submit(&self) -> bool {
        can_submit_to(
            &self.pending_submissions,
            self.composer_sessions.current_target(),
        )
    }

    pub(crate) fn handle_composer_escape(&mut self, cx: &mut Context<Self>) {
        let (action, arm) = composer_escape(
            self.snapshot.conversation.running,
            !self.snapshot.conversation.queue.steering.is_empty(),
            self.composer_sessions.current_target(),
            self.composer_escape_armed.as_ref(),
            Instant::now(),
        );
        self.composer_escape_armed = arm;
        match action {
            ComposerEscapeAction::ApplySteering => self.send(RuntimeCommand::ApplySteering, cx),
            ComposerEscapeAction::Abort => self.send(RuntimeCommand::Abort, cx),
            ComposerEscapeAction::None => {}
        }
    }

    pub(crate) fn enter_mode(&self) -> PromptMode {
        prompt_mode_for_enter(self.snapshot.conversation.running)
    }

    pub(crate) fn submit_follow_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.composer.read(cx).value().trim().to_owned();
        if !value.is_empty() || self.has_composer_attachments() {
            let mode = prompt_mode_for_follow_up(self.snapshot.conversation.running);
            self.submit(value, mode, window, cx);
        }
    }

    pub(in crate::app) fn resolve_pending_submission(
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
                self.save_composer_attachments(&target);
                continue;
            }

            let restored = if pending.append_on_failure {
                self.capture_composer_session(cx);
                Some(
                    self.composer_sessions
                        .append_to_draft(&target, &pending.text),
                )
            } else {
                self.composer_sessions
                    .restore_submitted_text(&target, pending.text.clone())
            };
            if !pending.images.is_empty() {
                let images = self.composer_images.entry(target.clone()).or_default();
                images.splice(0..0, pending.images.clone());
            }
            if !pending.pastes.is_empty() {
                let pastes = self.composer_pastes.entry(target.clone()).or_default();
                pastes.splice(0..0, pending.pastes.clone());
            }
            self.save_composer_attachments(&target);
            if let Some(snapshot) = restored
                && self.composer_sessions.current_target() == target
            {
                self.apply_composer_snapshot(snapshot, window, cx);
            }
            if let Some(session_key) = rejected_attachment_target(
                &pending.text,
                !pending.images.is_empty() || !pending.pastes.is_empty(),
                &target,
                self.composer_sessions.current_target(),
                session.as_deref(),
            ) {
                self.composer_sessions.promote(&target, session_key.clone());
                self.promote_center_surface(&target, &session_key);
                self.promote_composer_images(&target, &session_key);
                self.promote_composer_pastes(&target, &session_key);
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

pub(in crate::app) fn inactive_session_for_target(
    target: &str,
    selected_session: Option<&Path>,
    sessions: &[SessionSummary],
) -> Option<std::path::PathBuf> {
    let selected = selected_session?;
    (target == session_target(selected))
        .then(|| {
            sessions
                .iter()
                .find(|session| session.path == selected && session.archived)
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

fn submission_delivery(
    value: &str,
    requested: PromptMode,
    commands: &[crate::protocol::SlashCommand],
) -> (PromptMode, bool) {
    if super::slash_commands::exact(value.trim_start(), commands).is_some() {
        (PromptMode::Normal, true)
    } else {
        (requested, false)
    }
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

#[derive(Debug, PartialEq, Eq)]
enum ComposerEscapeAction {
    None,
    ApplySteering,
    Abort,
}

fn composer_escape(
    running: bool,
    has_queued_steer: bool,
    current_target: &str,
    armed: Option<&(String, Instant)>,
    now: Instant,
) -> (ComposerEscapeAction, Option<(String, Instant)>) {
    if !running {
        return (ComposerEscapeAction::None, None);
    }
    let armed_here = armed.is_some_and(|(target, at)| {
        target == current_target && now.saturating_duration_since(*at) <= COMPOSER_ABORT_DOUBLE_TAP
    });
    if armed_here {
        (ComposerEscapeAction::Abort, None)
    } else {
        let action = if has_queued_steer {
            ComposerEscapeAction::ApplySteering
        } else {
            ComposerEscapeAction::None
        };
        (action, Some((current_target.to_owned(), now)))
    }
}

#[cfg(test)]
#[path = "submissions_tests.rs"]
mod tests;
