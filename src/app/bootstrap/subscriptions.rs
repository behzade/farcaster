use super::*;

pub(super) struct BootstrapSubscriptions {
    pub(super) composer: Subscription,
    pub(super) search: Subscription,
    pub(super) session_title: Subscription,
    pub(super) window_activation: Subscription,
    pub(super) window_placement: Subscription,
}

pub(super) fn create(
    inputs: &inputs::BootstrapInputs,
    window: &mut Window,
    cx: &mut Context<FarcasterApp>,
) -> BootstrapSubscriptions {
    let composer = subscribe_composer(&inputs.composer, window, cx);
    let search = cx.subscribe_in(
        &inputs.search,
        window,
        |this, state, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                let query = state.read(cx).value().trim().to_owned();
                this.send(RuntimeCommand::LoadSessions(query), cx);
            }
        },
    );
    let session_title = cx.subscribe_in(
        &inputs.session_title,
        window,
        |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::PressEnter { .. } | InputEvent::Blur => {
                this.commit_session_title_edit(window, cx);
            }
            InputEvent::Change | InputEvent::Focus => {}
        },
    );
    let window_activation = cx.observe_window_activation(window, |this, window, cx| {
        let visible = session_shortcuts_visible_for_window(
            this.session_rail_view.read(cx).shortcuts_visible(),
            window.is_window_active(),
        );
        this.set_session_shortcuts_visible(visible, cx);
    });
    let window_placement = launch::observe_window_placement(window, cx);

    BootstrapSubscriptions {
        composer,
        search,
        session_title,
        window_activation,
        window_placement,
    }
}

fn subscribe_composer(
    composer: &Entity<TextareaState>,
    window: &mut Window,
    cx: &mut Context<FarcasterApp>,
) -> Subscription {
    cx.subscribe_in(
        composer,
        window,
        |this, state, event: &InputEvent, window, cx| match event {
            InputEvent::Change => {
                this.composer_view.update(cx, |view, _| {
                    view.reset_suggestion_selection();
                });
                this.composer_sessions.exit_history();
                let snapshot = input_snapshot(state.read(cx));
                let has_mention =
                    file_mentions::query_at_cursor(&snapshot.text, snapshot.cursor).is_some();
                this.composer_sessions.capture_current(snapshot);
                if has_mention {
                    this.request_composer_project_files(cx);
                }
                this.notify_composer(cx);
            }
            InputEvent::Blur => {
                this.composer_sessions
                    .capture_current(input_snapshot(state.read(cx)));
            }
            InputEvent::PressEnter { shift: false, .. } => {
                let input = state.read(cx);
                let value = input.value();
                if let Some(completion) = composer_completion::resolve_for_harness(
                    &value,
                    input.cursor(),
                    &this.composer_project_files,
                    this.composer_view.read(cx).suggestion_selection(),
                    &this.snapshot.commands,
                    this.active_harness(),
                ) {
                    let submitted_value = completion
                        .submit
                        .then(|| completion.snapshot.text.trim().to_owned());
                    this.apply_composer_snapshot(completion.snapshot, window, cx);
                    if let Some(value) = submitted_value {
                        this.submit(value, this.enter_mode(), window, cx);
                    } else {
                        this.composer_focus.focus(window, cx);
                    }
                } else {
                    let value = value.trim().to_owned();
                    if !value.is_empty() || this.has_composer_attachments() {
                        this.submit(value, this.enter_mode(), window, cx);
                    }
                }
            }
            InputEvent::PressEnter { .. } | InputEvent::Focus => {}
        },
    )
}
