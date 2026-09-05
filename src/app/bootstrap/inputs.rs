use super::*;

pub(super) struct BootstrapInputs {
    pub(super) composer: Entity<TextareaState>,
    pub(super) composer_focus: FocusHandle,
    pub(super) search: Entity<InputState>,
    pub(super) search_focus: FocusHandle,
    pub(super) session_title: Entity<InputState>,
    pub(super) network_proxy: Entity<InputState>,
    pub(super) dialog: Entity<TextareaState>,
    pub(super) dialog_focus: FocusHandle,
}

pub(super) fn create(
    composer_sessions: &ComposerSessions,
    saved_proxy: Option<&str>,
    window: &mut Window,
    cx: &mut Context<FarcasterApp>,
) -> BootstrapInputs {
    let composer = cx.new(|cx| {
        TextareaState::new(window, cx)
            .auto_grow(1, 8)
            .submit_on_enter(true)
            .placeholder("What would you like to work on?")
    });
    let initial_composer = composer_sessions.current();
    composer.update(cx, |input, cx| {
        input.set_value(initial_composer.text.clone(), window, cx);
        input.set_selected_range(initial_composer.restore_range(), cx);
    });
    let composer_focus = composer.read(cx).focus_handle(cx);

    let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search sessions"));
    let search_focus = search.read(cx).focus_handle(cx);
    let session_title = cx.new(|cx| InputState::new(window, cx));
    let network_proxy = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder("http://127.0.0.1:8080")
            .default_value(saved_proxy.unwrap_or_default())
    });
    let dialog = cx.new(|cx| {
        TextareaState::new(window, cx)
            .auto_grow(2, 12)
            .submit_on_enter(false)
    });

    BootstrapInputs {
        composer,
        composer_focus,
        search,
        search_focus,
        session_title,
        network_proxy,
        dialog,
        dialog_focus: cx.focus_handle(),
    }
}
