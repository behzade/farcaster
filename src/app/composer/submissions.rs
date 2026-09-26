use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use gpui::{Context, KeyDownEvent, Window};

use super::{
    ComposerImage, ComposerPaste, FarcasterApp, pastes as composer_pastes, prompt_fragments,
};
use crate::{
    app::composer::sessions::{ComposerSessions, ComposerSnapshot, session_target},
    app::composer::user_invocations,
    protocol::{PromptImage, PromptMode},
    runtime::RuntimeCommand,
    sessions::{SessionSummary, normalize_session_path},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::app) struct PendingSubmission {
    pub(in crate::app) id: String,
    pub(in crate::app) submitted_at: Instant,
    // Runtime replies keep this target even after the draft is promoted.
    pub(in crate::app) submitted_target: String,
    pub(in crate::app) mode: PromptMode,
    pub(in crate::app) text: String,
    pub(in crate::app) images: Vec<ComposerImage>,
    pub(in crate::app) pastes: Vec<ComposerPaste>,
    pub(in crate::app) append_on_failure: bool,
    pub(in crate::app) result: Option<(crate::agents::PromptOutcome, Option<std::path::PathBuf>)>,
}

impl FarcasterApp {
    pub(crate) fn submit(
        &mut self,
        value: String,
        mode: PromptMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_submit() || self.project.pending_trust_command.is_some() {
            return;
        }
        let Some(backend) = self.snapshot.harness else {
            let snapshot = Arc::make_mut(&mut self.snapshot);
            let index = snapshot.conversation.items.len();
            Arc::make_mut(&mut snapshot.conversation).push_local_error(
                "Prompt not sent",
                "Choose a backend before sending a message.".into(),
            );
            self.mark_transcript_changed(index, index == 0, cx);
            cx.notify();
            return;
        };
        let project = self.project.path.clone();
        if !self.ensure_backend_trust(backend, &project, window, cx) {
            return;
        }
        self.capture_composer_session(cx);
        let target = self.composer.sessions.current_target().to_owned();
        let editor_text = self.composer.input.read(cx).value().to_string();
        let (mode, allow_while_running) =
            submission_delivery(&value, mode, &self.snapshot.commands);
        let show_in_transcript = !self.snapshot.conversation.running;
        let images = self
            .composer
            .images
            .get(&target)
            .into_iter()
            .flatten()
            .map(|image| image.prompt.clone())
            .collect::<Vec<PromptImage>>();
        let pastes = self
            .composer
            .pastes
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
            &self.sessions.visible,
        );
        let submission_id = next_submission_id();
        match self.runtime.send(RuntimeCommand::Prompt {
            submission_id: submission_id.clone(),
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
                self.composer.sessions.record_submission(&target, &value);
                let pending_images = self.composer.images.remove(&target).unwrap_or_default();
                let pending_pastes = self.composer.pastes.remove(&target).unwrap_or_default();
                let transcript_images = show_in_transcript.then(|| {
                    Arc::new(
                        pending_images
                            .iter()
                            .filter_map(|image| {
                                crate::app::ui::images::from_preview(image.preview.clone())
                            })
                            .collect(),
                    )
                });
                self.composer.pending_submissions.insert(
                    submission_id.clone(),
                    PendingSubmission {
                        id: submission_id,
                        submitted_at: Instant::now(),
                        submitted_target: target.clone(),
                        mode,
                        text: editor_text.clone(),
                        images: pending_images,
                        pastes: pending_pastes,
                        append_on_failure: false,
                        result: None,
                    },
                );
                if self
                    .composer
                    .sessions
                    .clear_submitted_text(&target, &editor_text)
                    && self.composer.sessions.current_target() == target
                {
                    self.apply_composer_snapshot(ComposerSnapshot::default(), window, cx);
                }
                let snapshot = Arc::make_mut(&mut self.snapshot);
                let index = snapshot.conversation.items.len();
                let conversation = Arc::make_mut(&mut snapshot.conversation);
                if let Some(transcript_images) = transcript_images {
                    match (display_message, invocation) {
                        (Some(display), Some(invocation)) => {
                            conversation.push_local_user_with_images(
                                display,
                                transcript_images,
                                Some(invocation),
                            );
                        }
                        _ => {
                            let invocation =
                                user_invocations::contains_invocation(&value, &snapshot.commands);
                            conversation.push_local_user_with_images(
                                message,
                                transcript_images,
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
        true
    }

    pub(crate) fn handle_composer_escape(&mut self, cx: &mut Context<Self>) {
        let target = self.composer.sessions.current_target();
        let (action, arm) = composer_escape(
            self.snapshot.conversation.running,
            !self.snapshot.conversation.queue.steering.is_empty(),
            !self.snapshot.conversation.queue.follow_up.is_empty(),
            has_pending_submission(&self.composer.pending_submissions, target),
            target,
            self.composer.escape_armed.as_ref(),
            Instant::now(),
        );
        self.composer.escape_armed = arm;
        match action {
            ComposerEscapeAction::ApplySteering => self.send(RuntimeCommand::ApplySteering, cx),
            ComposerEscapeAction::Abort => self.send(RuntimeCommand::Abort, cx),
            ComposerEscapeAction::None => {}
        }
    }

    pub(in crate::app) fn handle_composer_escape_key(
        &mut self,
        event: &KeyDownEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key_action = composer_escape_key(event);
        let owns_escape = self.workspace.surface == crate::app::AppSurface::Chat
            && !self.native_workspace_covered_by_overlay()
            && self.keyboard_overlay_focus(window, cx).is_none()
            && self.composer.focus.contains_focused(window, cx)
            && key_action != ComposerEscapeKeyAction::Ignore;
        if !owns_escape {
            return false;
        }
        if key_action == ComposerEscapeKeyAction::Dispatch {
            self.handle_composer_escape(cx);
        }
        true
    }

    pub(crate) fn enter_mode(&self) -> PromptMode {
        prompt_mode_for_enter(self.snapshot.conversation.running)
    }

    pub(crate) fn submit_follow_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.composer.input.read(cx).value().trim().to_owned();
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
        let completed = take_resolved_pending_submissions(&mut self.composer.pending_submissions);
        for (target, pending, outcome, session) in completed {
            if !restores_composer(outcome) {
                self.save_composer_attachments(&target);
                continue;
            }

            // Preserve any newer draft, then restore each rejection in the
            // submission order established above.
            self.capture_composer_session(cx);
            let restored = restore_rejected_text(&mut self.composer.sessions, &target, &pending);
            self.composer
                .images
                .entry(target.clone())
                .or_default()
                .extend(pending.images.iter().cloned());
            self.composer
                .pastes
                .entry(target.clone())
                .or_default()
                .extend(pending.pastes.iter().cloned());
            self.save_composer_attachments(&target);
            if let Some(snapshot) = restored
                && self.composer.sessions.current_target() == target
            {
                self.apply_composer_snapshot(snapshot, window, cx);
            }
            if let Some(session_key) = rejected_attachment_target(
                &pending.text,
                !pending.images.is_empty() || !pending.pastes.is_empty(),
                &target,
                self.composer.sessions.current_target(),
                session.as_deref(),
            ) {
                self.composer.sessions.promote(&target, session_key.clone());
                self.promote_center_surface(&target, &session_key);
                self.promote_composer_images(&target, &session_key);
                self.promote_composer_pastes(&target, &session_key);
            }
        }
    }
}

fn restore_rejected_text(
    sessions: &mut ComposerSessions,
    target: &str,
    pending: &PendingSubmission,
) -> Option<ComposerSnapshot> {
    if pending.text.is_empty() {
        return None;
    }
    if pending.append_on_failure {
        return Some(sessions.append_to_draft(target, &pending.text));
    }
    match sessions.restore_submitted_text(target, pending.text.clone()) {
        Some(snapshot) => Some(snapshot),
        None => Some(sessions.append_to_draft(target, &pending.text)),
    }
}

fn restores_composer(outcome: crate::agents::PromptOutcome) -> bool {
    outcome == crate::agents::PromptOutcome::RejectedBeforeAcceptance
}

pub(in crate::app) fn take_resolved_pending_submissions(
    pending: &mut std::collections::HashMap<String, PendingSubmission>,
) -> Vec<(
    String,
    PendingSubmission,
    crate::agents::PromptOutcome,
    Option<std::path::PathBuf>,
)> {
    let mut ordered = pending.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| submission_order(left.1, right.1));
    let mut waiting_targets = std::collections::HashSet::new();
    let completed = ordered
        .into_iter()
        .filter_map(|(id, submission)| {
            let Some(result) = submission.result.clone() else {
                waiting_targets.insert(submission.submitted_target.clone());
                return None;
            };
            (!waiting_targets.contains(&submission.submitted_target)).then(|| (id.clone(), result))
        })
        .collect::<Vec<_>>();
    completed
        .into_iter()
        .filter_map(|(id, (outcome, session))| {
            pending.remove(&id).map(|submission| {
                let target = submission.submitted_target.clone();
                (target, submission, outcome, session)
            })
        })
        .collect()
}

fn submission_order(left: &PendingSubmission, right: &PendingSubmission) -> std::cmp::Ordering {
    left.submitted_at
        .cmp(&right.submitted_at)
        .then_with(|| left.id.cmp(&right.id))
}

static NEXT_SUBMISSION_ORDER: AtomicU64 = AtomicU64::new(0);

fn next_submission_id() -> String {
    let order = NEXT_SUBMISSION_ORDER.fetch_add(1, Ordering::Relaxed);
    format!("{order:020}-{}", uuid::Uuid::new_v4())
}

pub(in crate::app) fn has_pending_submission(
    pending: &std::collections::HashMap<String, PendingSubmission>,
    target: &str,
) -> bool {
    pending
        .values()
        .any(|submission| submission.submitted_target == target)
}

// Presentation only: retain attachment information without changing the payload
// used for delivery or acknowledgement.
fn pending_queue_preview(submission: &PendingSubmission) -> String {
    let mut parts = Vec::new();
    match submission.images.len() {
        0 => {}
        1 => parts.push("1 image".to_owned()),
        count => parts.push(format!("{count} images")),
    }
    parts.extend(submission.pastes.iter().map(ComposerPaste::file_name));
    if parts.is_empty() {
        return submission.text.clone();
    }
    if !submission.text.trim().is_empty() {
        parts.push(submission.text.clone());
    }
    parts.join(" · ")
}

/// The queue shown by the production composer. A pending submission first
/// appears locally, then the backend queue begins reflecting that same item
/// after admission. Overlay matching local entries onto native entries so this
/// handoff does not briefly render one submission twice.
pub(in crate::app) fn visible_prompt_queue(
    native: &crate::conversation::QueueState,
    pending: &std::collections::HashMap<String, PendingSubmission>,
    target: &str,
) -> crate::conversation::QueueState {
    let mut visible = native.clone();
    visible
        .steering_ids
        .resize(visible.steering.len(), String::new());
    visible
        .follow_up_ids
        .resize(visible.follow_up.len(), String::new());
    let mut submissions = pending
        .values()
        .filter(|submission| submission.submitted_target == target && submission.result.is_none())
        .collect::<Vec<_>>();
    submissions.sort_by(|left, right| submission_order(left, right));
    let mut matched_steering = vec![false; visible.steering.len()];
    let mut matched_follow_up = vec![false; visible.follow_up.len()];
    for submission in submissions {
        let (messages, matched, ids) = match submission.mode {
            PromptMode::Steer => (
                &mut visible.steering,
                &mut matched_steering,
                &mut visible.steering_ids,
            ),
            PromptMode::FollowUp => (
                &mut visible.follow_up,
                &mut matched_follow_up,
                &mut visible.follow_up_ids,
            ),
            PromptMode::Normal => continue,
        };
        let preview = pending_queue_preview(submission);
        if let Some(index) = messages.iter().enumerate().find_map(|(index, message)| {
            // Submission trims the editor value; the local copy retains it for recovery.
            (!matched[index] && (message == &submission.text || message == submission.text.trim()))
                .then_some(index)
        }) {
            matched[index] = true;
            messages[index] = preview;
            if ids[index].is_empty() {
                ids[index] = submission.id.clone();
            }
        } else {
            messages.push(preview);
            ids.push(submission.id.clone());
            matched.push(true);
        }
    }
    visible
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
    has_queued_follow_up: bool,
    has_pending_submission: bool,
    current_target: &str,
    armed: Option<&(String, Instant)>,
    now: Instant,
) -> (ComposerEscapeAction, Option<(String, Instant)>) {
    let armed_here = armed.is_some_and(|(target, at)| {
        target == current_target && now.saturating_duration_since(*at) <= COMPOSER_ABORT_DOUBLE_TAP
    });
    if armed_here {
        (ComposerEscapeAction::Abort, None)
    } else if has_queued_steer || has_queued_follow_up || has_pending_submission {
        (
            ComposerEscapeAction::ApplySteering,
            Some((current_target.to_owned(), now)),
        )
    } else if running {
        (
            ComposerEscapeAction::None,
            Some((current_target.to_owned(), now)),
        )
    } else {
        (ComposerEscapeAction::None, None)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ComposerEscapeKeyAction {
    Ignore,
    Consume,
    Dispatch,
}

fn composer_escape_key(event: &KeyDownEvent) -> ComposerEscapeKeyAction {
    if event.keystroke.modifiers.modified() || !event.keystroke.key.eq_ignore_ascii_case("escape") {
        ComposerEscapeKeyAction::Ignore
    } else if event.is_held {
        ComposerEscapeKeyAction::Consume
    } else {
        ComposerEscapeKeyAction::Dispatch
    }
}

#[cfg(test)]
#[path = "submissions_tests.rs"]
mod tests;
