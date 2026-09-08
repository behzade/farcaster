use gpui::{Context, PromptButton, PromptLevel, Window};

use super::FarcasterApp;
use crate::app::session::activity::{snapshot_has_active_work, status_has_active_work};
use crate::protocol::BackgroundJobState;

impl FarcasterApp {
    pub(crate) fn request_application_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active = self
            .run_statuses
            .values()
            .any(|status| status_has_active_work(status))
            || snapshot_has_active_work(&self.snapshot)
            || !self.pending_submissions.is_empty()
            || self.all_sessions.iter().any(|session| session.is_running)
            || self.background_jobs.iter().any(|job| {
                matches!(
                    job.state,
                    BackgroundJobState::Starting | BackgroundJobState::Running
                )
            });
        if !active {
            cx.quit();
            return;
        }

        let answer = window.prompt(
            PromptLevel::Warning,
            "Exit Farcaster?",
            Some("Agents, subagents, or tool runs are still active. Exiting may interrupt this work."),
            &[PromptButton::cancel("Cancel"), PromptButton::ok("Exit")],
            cx,
        );
        cx.spawn(async move |_, cx| {
            if answer.await == Ok(1) {
                cx.update(|cx| cx.quit());
            }
        })
        .detach();
    }
}
