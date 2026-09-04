use super::*;

impl FarcasterApp {
    pub(in crate::app) fn switch_composer_target(
        &mut self,
        target: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = input_snapshot(self.composer.read(cx));
        let current_target = self.composer_sessions.current_target().to_owned();
        let discard = self.sync_current_draft(&current, &current_target);
        let snapshot = if discard {
            self.session_surfaces.remove(&current_target);
            self.composer_sessions
                .discard_and_switch(&current_target, target)
        } else {
            self.capture_center_surface();
            self.composer_sessions.switch_to(target, current)
        };
        self.apply_composer_snapshot(snapshot, window, cx);
    }

    pub(in crate::app) fn capture_composer_session(&mut self, cx: &mut Context<Self>) {
        self.composer_sessions
            .capture_current(input_snapshot(self.composer.read(cx)));
    }

    pub(in crate::app) fn apply_composer_snapshot(
        &self,
        snapshot: ComposerSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = snapshot.restore_range();
        let text = snapshot.text;
        self.composer.update(cx, |input, cx| {
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
        let current = input_snapshot(self.composer.read(cx));
        match self.composer_sessions.navigate_history(key, current) {
            HistoryNavigation::PassThrough => false,
            HistoryNavigation::Handled(snapshot) => {
                if let Some(snapshot) = snapshot {
                    self.apply_composer_snapshot(snapshot, window, cx);
                }
                true
            }
        }
    }

    pub(in crate::app) fn select_model(&mut self, model: &Model, cx: &mut Context<Self>) {
        self.send(RuntimeCommand::SetModel(model.clone()), cx);
        cx.notify();
    }

    pub(in crate::app) fn set_thinking_level(&mut self, level: String, cx: &mut Context<Self>) {
        self.send(RuntimeCommand::SetThinking(level), cx);
        cx.notify();
    }

    pub(in crate::app) fn set_agent_mode(&mut self, mode: String, cx: &mut Context<Self>) {
        self.send(RuntimeCommand::SetMode(mode), cx);
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
