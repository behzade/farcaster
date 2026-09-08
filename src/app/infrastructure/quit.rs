use std::{cell::RefCell, rc::Rc};

use gpui::{App, Context, PromptButton, PromptLevel, WeakEntity, Window};

use super::{FarcasterApp, QuitApplication};
use crate::app::session::activity::{snapshot_has_active_work, status_has_active_work};
use crate::protocol::BackgroundJobState;

pub(super) fn install<T: 'static>(
    app: Rc<RefCell<Option<WeakEntity<T>>>>,
    request_quit: fn(&mut T, &mut Window, &mut Context<T>),
    cx: &mut App,
) {
    cx.on_action(move |_: &QuitApplication, cx| {
        let app = app.clone();
        // Key dispatch already holds the window; wait until it is available again.
        cx.defer(move |cx| {
            let Some(app) = app.borrow().clone() else {
                cx.quit();
                return;
            };
            if let Err(error) = app.update_in(cx, request_quit) {
                zlog::error!("Could not check active work before quitting: {error}");
            }
        });
    });
}

pub(super) fn install_window(window: &Window, cx: &App) {
    window.on_window_should_close(cx, |_, cx| {
        cx.dispatch_action(&QuitApplication);
        false
    });
}

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

#[cfg(test)]
#[path = "quit_tests.rs"]
mod tests;
