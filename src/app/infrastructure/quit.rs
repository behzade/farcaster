use std::{cell::RefCell, rc::Rc};

use gpui::{App, Context, FocusHandle, WeakEntity, Window};

use super::{FarcasterApp, QuitApplication};
use crate::app::session::activity::application_has_active_work;

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
        if self.lifecycle.saving_before_quit {
            return;
        }
        if let Some(pending) = &self.lifecycle.pending_quit {
            pending.focus.focus(window, cx);
            return;
        }

        let active = application_has_active_work(
            &self.activity.run_statuses,
            &self.snapshot,
            &self.composer.pending_submissions,
            &self.sessions.all,
            &self.activity.background_jobs,
        );
        if !active {
            self.quit_after_saving(cx);
            return;
        }

        self.cover_native_workspace_surface(cx);
        let pending = PendingQuit {
            focus: cx.focus_handle(),
            return_focus: window.focused(cx),
        };
        pending.focus.focus(window, cx);
        self.lifecycle.pending_quit = Some(pending);
        cx.notify();
    }

    pub(in crate::app) fn close_quit_confirmation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.lifecycle.pending_quit.take() else {
            return;
        };
        self.restore_overlay_focus(pending.return_focus, &pending.focus, window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
    }

    pub(in crate::app) fn confirm_application_quit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.lifecycle.pending_quit.is_some() {
            self.close_quit_confirmation(window, cx);
            self.quit_after_saving(cx);
        }
    }

    fn quit_after_saving(&mut self, cx: &mut Context<Self>) {
        self.capture_composer_session(cx);
        let target = self.composer.sessions.current_target().to_owned();
        if !self.sync_current_draft(&target) {
            self.lifecycle.saving_before_quit = false;
            self.notify_session_rail(cx);
            return;
        }
        self.lifecycle.saving_before_quit = true;
        let revision = self.sessions.writer.revision();
        let flush = self.sessions.writer.flush();
        cx.spawn(async move |weak, cx| {
            let result = flush.await;
            let _ = weak.update(cx, |app, cx| match result {
                Ok(()) if app.sessions.writer.revision() == revision => {
                    app.lifecycle.saving_before_quit = false;
                    cx.quit();
                }
                Ok(()) => app.quit_after_saving(cx),
                Err(error) => {
                    app.lifecycle.saving_before_quit = false;
                    app.sessions.error = Some(error);
                    app.notify_session_rail(cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
#[path = "quit_tests.rs"]
mod tests;
