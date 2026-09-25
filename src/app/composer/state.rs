use super::*;

impl FarcasterApp {
    pub(in crate::app) fn switch_composer_target(
        &mut self,
        target: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigation.chat.activation.clear();
        let current = input_snapshot(self.composer.input.read(cx));
        let current_target = self.composer.sessions.current_target().to_owned();
        let discard = self.sync_current_draft(&current, &current_target);
        let snapshot = if discard {
            self.workspace.session_surfaces.remove(&current_target);
            self.workspace.editor.session_tabs.remove(&current_target);
            self.composer
                .sessions
                .discard_and_switch(&current_target, target)
        } else {
            self.capture_center_surface();
            self.composer.sessions.switch_to(target, current)
        };
        self.apply_composer_snapshot(snapshot, window, cx);
    }

    pub(in crate::app) fn capture_composer_session(&mut self, cx: &mut Context<Self>) {
        self.composer
            .sessions
            .capture_current(input_snapshot(self.composer.input.read(cx)));
    }

    pub(in crate::app) fn apply_composer_snapshot(
        &self,
        snapshot: ComposerSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = snapshot.restore_range();
        let text = snapshot.text;
        self.composer.input.update(cx, |input, cx| {
            input.set_value(text, window, cx);
            input.set_selected_range(range, cx);
        });
    }

    pub(in crate::app) fn handle_composer_history_key(
        &mut self,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let current = input_snapshot(self.composer.input.read(cx));
        match self.composer.sessions.navigate_history(key, current) {
            HistoryNavigation::PassThrough => false,
            HistoryNavigation::Handled(snapshot) => {
                if let Some(snapshot) = snapshot {
                    self.apply_composer_snapshot(snapshot, window, cx);
                }
                true
            }
        }
    }

    #[cfg(test)]
    pub(in crate::app) fn select_model(&mut self, model: &Model, cx: &mut Context<Self>) {
        self.send(RuntimeCommand::SetModel(model.clone()), cx);
        cx.notify();
    }

    pub(in crate::app) fn select_model_from_ui(
        &mut self,
        model: &Model,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_model_with_effort(model, None, false, window, cx);
    }

    pub(in crate::app) fn select_model_with_effort(
        &mut self,
        model: &Model,
        effort: Option<String>,
        apply_effort: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let available = crate::agents::available_access_modes(
            self.snapshot.harness,
            Some(self.snapshot.catalog_model(model)),
            self.snapshot.sandbox_adapter.as_deref(),
        );
        if available.is_empty() {
            self.notify_workspace_error(
                "Model",
                "No access mode is available for this model".into(),
                cx,
            );
            return false;
        }
        if !available.contains(&self.snapshot.access_mode) {
            if self.workspace.runtime_picker.open {
                self.set_runtime_picker_open(false, window, cx);
            }
            self.cover_native_workspace_surface(cx);
            let pending = crate::app::navigation::PendingModelAccess {
                focus: cx.focus_handle(),
                model: model.clone(),
                modes: available,
                effort,
                apply_effort,
                return_focus: window.focused(cx),
            };
            pending.focus.focus(window, cx);
            self.navigation.pending_model_access = Some(pending);
            cx.notify();
            return false;
        }
        self.send_model_selection(
            model.clone(),
            effort,
            apply_effort,
            self.snapshot.access_mode,
            cx,
        );
        true
    }

    fn send_model_selection(
        &mut self,
        model: Model,
        effort: Option<String>,
        apply_effort: bool,
        access_mode: crate::runtime::HarnessAccessMode,
        cx: &mut Context<Self>,
    ) {
        self.send(
            RuntimeCommand::SetModelWithAccessMode { model, access_mode },
            cx,
        );
        if apply_effort
            && (effort.is_some() || crate::agents::supports_reasoning_reset(self.snapshot.harness))
        {
            self.set_thinking_level(effort, cx);
        }
        cx.notify();
    }

    pub(in crate::app) fn close_model_access_confirmation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<crate::app::navigation::PendingModelAccess> {
        let pending = self.navigation.pending_model_access.take()?;
        self.restore_overlay_focus(pending.return_focus.clone(), &pending.focus, window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
        Some(pending)
    }

    pub(in crate::app) fn confirm_model_access(
        &mut self,
        access_mode: crate::runtime::HarnessAccessMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.navigation.pending_model_access.as_ref() else {
            return;
        };
        let model = self
            .snapshot
            .models
            .iter()
            .find(|model| model.provider == pending.model.provider && model.id == pending.model.id)
            .cloned();
        let Some(model) = model else {
            self.close_model_access_confirmation(window, cx);
            self.notify_workspace_error("Model", "This model is no longer available".into(), cx);
            return;
        };
        let available = crate::agents::available_access_modes(
            self.snapshot.harness,
            Some(&model),
            self.snapshot.sandbox_adapter.as_deref(),
        );
        if available.is_empty() {
            self.close_model_access_confirmation(window, cx);
            self.notify_workspace_error(
                "Model",
                "No access mode is available for this model".into(),
                cx,
            );
            return;
        }
        if !available.contains(&access_mode) {
            if let Some(pending) = self.navigation.pending_model_access.as_mut() {
                pending.modes = available;
            }
            cx.notify();
            return;
        }
        let Some(pending) = self.close_model_access_confirmation(window, cx) else {
            return;
        };
        if self.navigation.picker.is_some() {
            self.close_picker(window, cx);
        }
        self.send_model_selection(model, pending.effort, pending.apply_effort, access_mode, cx);
    }

    pub(in crate::app) fn set_thinking_level(
        &mut self,
        level: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.send(
            match level {
                Some(level) => RuntimeCommand::SetThinking(level),
                None => RuntimeCommand::ResetThinking,
            },
            cx,
        );
        cx.notify();
    }

    pub(in crate::app) fn set_service_tier(&mut self, tier: String, cx: &mut Context<Self>) {
        self.send(RuntimeCommand::SetServiceTier(tier), cx);
        cx.notify();
    }

    pub(in crate::app) fn set_access_mode(
        &mut self,
        level: crate::runtime::HarnessAccessMode,
        cx: &mut Context<Self>,
    ) {
        self.send(RuntimeCommand::SetAccessMode(level), cx);
        cx.notify();
    }
}
