//! Composer submission lifecycle, including image attachment ownership.

use std::sync::Arc;

use gpui::{Context, Window};

use super::PiApp;
use crate::{
    composer_sessions::ComposerSnapshot,
    protocol::{PromptImage, PromptMode},
    runtime::RuntimeCommand,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct PendingSubmission {
    pub(super) target: String,
    pub(super) text: String,
    pub(super) images: Vec<PromptImage>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SubmissionResolution {
    ClearEditor,
    KeepEditor,
    Ignore,
}

impl PiApp {
    pub(crate) fn submit(&mut self, value: String, mode: PromptMode, cx: &mut Context<Self>) {
        if !self.can_submit() {
            return;
        }
        let target = self.composer_sessions.current_target().to_owned();
        let editor_text = self.composer.read(cx).value().to_string();
        let images = self
            .composer_images
            .get(&target)
            .into_iter()
            .flatten()
            .map(|image| image.prompt.clone())
            .collect::<Vec<_>>();
        match self.runtime.send(RuntimeCommand::Prompt {
            target: target.clone(),
            mode,
            message: value.clone(),
            images: images.clone(),
        }) {
            Ok(()) => {
                self.composer_sessions.record_submission(&target, &value);
                self.pending_submission = Some(PendingSubmission {
                    target,
                    text: editor_text,
                    images,
                });
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

    pub(crate) fn can_submit(&self) -> bool {
        self.pending_submission.is_none()
    }

    pub(crate) fn enter_mode(&self) -> PromptMode {
        prompt_mode_for_enter(self.snapshot.conversation.running)
    }

    pub(crate) fn submit_follow_up(&mut self, cx: &mut Context<Self>) {
        let value = self.composer.read(cx).value().trim().to_owned();
        if !value.is_empty() || self.has_composer_images() {
            self.submit(value, PromptMode::FollowUp, cx);
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
        let current = self.composer_sessions.snapshot_for(&target).text;
        let resolution = submission_resolution(
            self.pending_submission.as_ref(),
            &target,
            accepted,
            &current,
        );
        if resolution == SubmissionResolution::Ignore {
            return;
        }
        let Some(pending) = self.pending_submission.take() else {
            return;
        };
        if accepted
            && self
                .composer_images
                .get(&pending.target)
                .map(|images| {
                    images
                        .iter()
                        .map(|image| &image.prompt)
                        .eq(pending.images.iter())
                })
                .unwrap_or(pending.images.is_empty())
        {
            self.composer_images.remove(&pending.target);
        }
        if resolution == SubmissionResolution::ClearEditor {
            let cleared = self
                .composer_sessions
                .clear_submitted_text(&pending.target, &pending.text);
            if cleared && self.composer_sessions.current_target() == pending.target {
                self.apply_composer_snapshot(ComposerSnapshot::default(), window, cx);
            }
        }
    }
}

fn submission_resolution(
    pending: Option<&PendingSubmission>,
    target: &str,
    accepted: bool,
    current_text: &str,
) -> SubmissionResolution {
    let Some(pending) = pending.filter(|pending| pending.target == target) else {
        return SubmissionResolution::Ignore;
    };
    if accepted && current_text == pending.text {
        SubmissionResolution::ClearEditor
    } else {
        SubmissionResolution::KeepEditor
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
    fn accepted_submission_clears_only_the_unchanged_editor() {
        let pending = pending();
        assert_eq!(
            submission_resolution(Some(&pending), "session:test", true, "submitted"),
            SubmissionResolution::ClearEditor
        );
        assert_eq!(
            submission_resolution(Some(&pending), "session:test", true, "new draft"),
            SubmissionResolution::KeepEditor
        );
    }

    #[test]
    fn rejected_or_stale_submission_never_clears_the_editor() {
        let pending = pending();
        assert_eq!(
            submission_resolution(Some(&pending), "session:test", false, "submitted"),
            SubmissionResolution::KeepEditor
        );
        assert_eq!(
            submission_resolution(Some(&pending), "session:other", true, "submitted"),
            SubmissionResolution::Ignore
        );
    }

    #[test]
    fn enter_prompts_when_idle_and_steers_while_running() {
        assert_eq!(prompt_mode_for_enter(false), PromptMode::Normal);
        assert_eq!(prompt_mode_for_enter(true), PromptMode::Steer);
    }
}
