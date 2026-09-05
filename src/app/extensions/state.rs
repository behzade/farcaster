use super::*;
use crate::app::*;

impl FarcasterApp {
    // App-owned failures use the notification overlay, never transcript events or
    // center-surface navigation. Callers must validate async ownership first.
    pub(in crate::app) fn notify_workspace_error(
        &mut self,
        source: &str,
        error: String,
        cx: &mut Context<Self>,
    ) {
        zlog::warn!("{source}: {error}");
        self.extension.push_notification(
            format!("workspace:{source}"),
            format!("{source}: {error}"),
            crate::protocol::NotifyTone::Error,
        );
        self.sync_notification_expiries(cx);
        cx.notify();
    }

    pub(in crate::app) fn record_run_status(
        &mut self,
        target: String,
        status: String,
        force_recent: bool,
    ) -> bool {
        if status == "Done" {
            if starts_recent_completion(
                self.run_statuses.get(&target).map(String::as_str),
                &status,
                force_recent,
            ) {
                self.run_statuses.insert(target.clone(), status);
                self.recent_completions.insert(target, Instant::now());
                return true;
            }
            if self.recent_completions.contains_key(&target) {
                self.run_statuses.insert(target, status);
                return true;
            }
            self.run_statuses.remove(&target);
            self.recent_completions.remove(&target);
            return false;
        }
        self.recent_completions.remove(&target);
        self.run_statuses.insert(target, status);
        false
    }

    pub(in crate::app) fn reset_session_ui(
        &mut self,
        generation: u64,
        preserve_submission: bool,
        cx: &mut Context<Self>,
    ) {
        self.runtime_generation = generation;
        self.extension.reset();
        self.parked_extension = None;
        self.background_jobs.clear();
        self.restored_dialog_id = None;
        self.dismissed_restored_dialog_id = None;
        self.notification_expiries.clear();
        self.pending_dialog_setup = false;
        self.pending_title = Some((generation, "Pi".into()));
        self.pending_editor_text = None;
        self.post_render_focus = Some(PostRenderFocus::ActiveSurface(Some(
            self.composer_focus.clone(),
        )));
        self.dialog_return_focus = None;
        self.overlays.sessions = false;
        self.overlays.run = false;
        self.sheet_return_focus = None;
        self.overlays.pending_setup = false;
        if !preserve_submission {
            self.reset_transcript_ui(cx);
        }
    }

    pub(in crate::app) fn sync_restored_dialog(&mut self) {
        let Some(request) = self.snapshot.pending_question.clone() else {
            self.clear_restored_dialog();
            return;
        };
        let Some(id) = request.dialog_id().map(str::to_owned) else {
            return;
        };
        if self.restored_dialog_id.as_deref() == Some(id.as_str()) {
            return;
        }
        self.clear_restored_dialog();
        if self.dismissed_restored_dialog_id.as_deref() == Some(id.as_str()) {
            return;
        }
        self.dismissed_restored_dialog_id = None;
        if self.extension.dialog.is_some() {
            return;
        }
        if matches!(self.extension.apply(request), ExtensionEffect::DialogOpened) {
            self.restored_dialog_id = Some(id);
            self.pending_dialog_setup = true;
        }
    }

    pub(in crate::app) fn clear_restored_dialog(&mut self) {
        if let Some(id) = self.restored_dialog_id.take() {
            let _ = self.extension.cancel(&id);
        }
    }

    pub(in crate::app) fn apply_extension_request(
        &mut self,
        request: ExtensionUiRequest,
        generation: u64,
        _cx: &mut Context<Self>,
    ) {
        match self.extension.apply(request) {
            ExtensionEffect::DialogOpened => self.pending_dialog_setup = true,
            ExtensionEffect::SetTitle(title) => self.pending_title = Some((generation, title)),
            ExtensionEffect::SetEditorText(text) => {
                self.pending_editor_text = Some((generation, text))
            }
            ExtensionEffect::PersistError(_) | ExtensionEffect::None => {}
            ExtensionEffect::Diagnostic(message) => {
                Arc::make_mut(&mut Arc::make_mut(&mut self.snapshot).conversation)
                    .diagnostics
                    .push(message)
            }
        }
    }

    pub(in crate::app) fn reset_transcript_ui(&mut self, cx: &mut Context<Self>) {
        self.transcript_view.update(cx, |transcript, cx| {
            transcript.reset();
            cx.notify();
        });
    }

    pub(crate) fn activate_system_notification(
        &mut self,
        tag: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if tag != SYSTEM_NOTIFICATION_TAG {
            return;
        }
        if let Some((path, project)) = self.system_notification_target.clone() {
            self.select_session(path, project, window, cx);
        }
    }
}
