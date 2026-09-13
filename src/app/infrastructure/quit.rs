use std::{cell::RefCell, rc::Rc};

use gpui::{App, Context, FocusHandle, WeakEntity, Window};

use super::{FarcasterApp, QuitApplication};
use crate::app::session::activity::{snapshot_has_active_work, status_has_active_work};
use crate::protocol::BackgroundJobState;

pub(in crate::app) struct PendingQuit {
    pub(in crate::app) focus: FocusHandle,
    return_focus: Option<FocusHandle>,
}

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
    window.on_window_should_close(cx, |window, cx| {
        // The close callback already holds this window; do not re-enter it via App.
        window.dispatch_action(Box::new(QuitApplication), cx);
        false
    });
}

impl FarcasterApp {
    pub(crate) fn request_application_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pending) = &self.pending_quit {
            pending.focus.focus(window, cx);
            return;
        }

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

        self.cover_native_workspace_surface(cx);
        let pending = PendingQuit {
            focus: cx.focus_handle(),
            return_focus: window.focused(cx),
        };
        pending.focus.focus(window, cx);
        self.pending_quit = Some(pending);
        cx.notify();
    }

    pub(in crate::app) fn close_quit_confirmation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.pending_quit.take() else {
            return;
        };
        self.restore_overlay_focus(pending.return_focus, &pending.focus, window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
    }

    pub(in crate::app) fn confirm_application_quit(&mut self, cx: &mut Context<Self>) {
        if self.pending_quit.take().is_some() {
            cx.quit();
        }
    }
}

#[cfg(test)]
#[path = "quit_tests.rs"]
mod tests;
