use super::*;
use crate::app::*;

impl FarcasterApp {
    pub(in crate::app) fn notify_workspace_error(
        &mut self,
        source: &str,
        error: String,
        cx: &mut Context<Self>,
    ) {
        zlog::warn!("{source}: {error}");
        self.extensions.active.push_notification(
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
                self.activity.run_statuses.get(&target).map(String::as_str),
                &status,
                force_recent,
            ) {
                self.activity.run_statuses.insert(target.clone(), status);
                self.activity
                    .recent_completions
                    .insert(target, Instant::now());
                return true;
            }
            if self.activity.recent_completions.contains_key(&target) {
                self.activity.run_statuses.insert(target, status);
                return true;
            }
            self.activity.run_statuses.remove(&target);
            self.activity.recent_completions.remove(&target);
            return false;
        }
        self.activity.recent_completions.remove(&target);
        self.activity.run_statuses.insert(target, status);
        false
    }

    pub(in crate::app) fn reset_session_ui(
        &mut self,
        generation: u64,
        preserve_submission: bool,
        cx: &mut Context<Self>,
    ) {
        self.runtime_generation = generation;
        if self.workspace.runtime_picker.worker.is_some() {
            self.workspace.runtime_picker = Default::default();
        }
        self.extensions.active.reset();
        self.extensions.parked = None;
        self.activity.background_jobs.clear();
        self.extensions.restored_dialog_id = None;
        self.extensions.dismissed_restored_dialog_id = None;
        self.extensions.notification_expiries.clear();
        self.extensions.pending_dialog_setup = false;
        self.extensions.pending_title = Some((generation, "Farcaster".into()));
        self.extensions.pending_editor_text = None;
        self.extensions.dialog_return_focus = None;
        self.overlays.view.sessions = false;
        self.overlays.view.run = false;
        self.overlays.sheet_return_focus = None;
        self.overlays.view.pending_setup = false;
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
        if self.extensions.restored_dialog_id.as_deref() == Some(id.as_str()) {
            return;
        }
        self.clear_restored_dialog();
        if self.extensions.dismissed_restored_dialog_id.as_deref() == Some(id.as_str()) {
            return;
        }
        self.extensions.dismissed_restored_dialog_id = None;
        if self.extensions.active.dialog.is_some() {
            return;
        }
        if matches!(
            self.extensions.active.apply(request),
            ExtensionEffect::DialogOpened
        ) {
            self.extensions.restored_dialog_id = Some(id);
            self.extensions.pending_dialog_setup = true;
        }
    }

    pub(in crate::app) fn clear_restored_dialog(&mut self) {
        if let Some(id) = self.extensions.restored_dialog_id.take() {
            let _ = self.extensions.active.cancel(&id);
        }
    }

    pub(in crate::app) fn apply_extension_request(
        &mut self,
        request: ExtensionUiRequest,
        generation: u64,
        _cx: &mut Context<Self>,
    ) {
        match self.extensions.active.apply(request) {
            ExtensionEffect::DialogOpened => self.extensions.pending_dialog_setup = true,
            ExtensionEffect::SetTitle(title) => {
                self.extensions.pending_title = Some((generation, title))
            }
            ExtensionEffect::SetEditorText(text) => {
                self.extensions.pending_editor_text = Some((generation, text))
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
        self.views.transcript.update(cx, |transcript, cx| {
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
        if let Some((path, project)) = self.activity.system_notification_targets.get(tag).cloned() {
            self.select_session(path, project, window, cx);
        }
    }

    pub(in crate::app) fn show_attention_notification(
        &mut self,
        title: &str,
        body: &str,
        target: Option<(PathBuf, PathBuf)>,
        cx: &mut Context<Self>,
    ) {
        let tag = target.as_ref().map_or_else(
            || SYSTEM_NOTIFICATION_TAG.to_owned(),
            |(path, _)| format!("{SYSTEM_NOTIFICATION_TAG}:{}", path.display()),
        );
        if let Some(target) = target {
            self.activity
                .system_notification_targets
                .insert(tag.clone(), target);
        }
        cx.show_system_notification(SystemNotification {
            tag: tag.into(),
            title: title.to_owned().into(),
            body: body.to_owned().into(),
            actions: Vec::new(),
        });
    }
}
