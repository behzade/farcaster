use super::*;

pub(super) struct BootstrapInputs {
    pub(super) composer: Entity<EditorState>,
    pub(super) composer_decorations: TextDecorationCollection,
    pub(super) composer_focus: FocusHandle,
    pub(super) search: Entity<InputState>,
    pub(super) search_focus: FocusHandle,
    pub(super) session_title: Entity<InputState>,
    pub(super) network_proxy: Entity<InputState>,
    pub(super) editor_command: Entity<InputState>,
    pub(super) harness_profile_name: Entity<InputState>,
    pub(super) harness_profile_executable: Entity<InputState>,
    pub(super) harness_profile_data_directory: Entity<InputState>,
    pub(super) dialog: Entity<TextareaState>,
    pub(super) dialog_focus: FocusHandle,
}

pub(super) fn create(
    composer_sessions: &ComposerSessions,
    saved_proxy: Option<&str>,
    saved_editor_command: &str,
    window: &mut Window,
    cx: &mut Context<FarcasterApp>,
) -> BootstrapInputs {
    let composer = cx.new(|cx| composer_input(window, cx));
    let initial_composer = composer_sessions.current();
    composer.update(cx, |input, cx| {
        input.set_value(initial_composer.text.clone(), window, cx);
        input.set_selected_range(initial_composer.restore_range(), cx);
    });
    let composer_decorations = composer.update(cx, |input, cx| {
        input.create_decorations_collection(Vec::new(), cx)
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
    let editor_command = cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder("micro -softwrap true")
            .default_value(saved_editor_command)
    });
    let harness_profile_name =
        cx.new(|cx| InputState::new(window, cx).placeholder("Name, e.g. Claudex"));
    let harness_profile_executable = cx.new(|cx| {
        InputState::new(window, cx).placeholder("Command or absolute path, e.g. claudex")
    });
    let harness_profile_data_directory = cx.new(|cx| {
        InputState::new(window, cx).placeholder("Optional data directory, e.g. ~/.codex2")
    });
    let dialog = cx.new(|cx| {
        TextareaState::new(window, cx)
            .auto_grow(2, 12)
            .submit_on_enter(false)
    });

    BootstrapInputs {
        composer,
        composer_decorations,
        composer_focus,
        search,
        search_focus,
        session_title,
        network_proxy,
        editor_command,
        harness_profile_name,
        harness_profile_executable,
        harness_profile_data_directory,
        dialog,
        dialog_focus: cx.focus_handle(),
    }
}

pub(in crate::app) fn composer_input(
    window: &mut Window,
    cx: &mut Context<EditorState>,
) -> EditorState {
    EditorState::new(window, cx)
        .language("plaintext")
        .line_number(false)
        .indent_guides(false)
        .folding(false)
        .auto_close(false)
        .smart_indent(false)
        .searchable(false)
        .soft_wrap(true)
        .scroll_beyond_last_line(Some(0))
        .submit_on_enter(true)
        .placeholder("What would you like to work on?")
}
