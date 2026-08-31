use gpui::{Context, PromptButton, PromptLevel, Window};

use super::FarcasterApp;
use crate::protocol::BackgroundJobState;

impl FarcasterApp {
    pub(crate) fn request_application_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let active = self.run_statuses.values().any(|status| status == "Working")
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
            "Exit Pi?",
            Some("Agents, subagents, or tool runs are still active. Exiting now will stop them."),
            &[PromptButton::ok("Exit"), PromptButton::cancel("Cancel")],
            cx,
        );
        cx.spawn(async move |_, cx| {
            if answer.await == Ok(0) {
                cx.update(|cx| cx.quit());
            }
        })
        .detach();
    }
}
