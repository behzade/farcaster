use super::*;

#[test]
fn search_matches_labels_details_and_keywords_by_term() {
    let row = PickerRow::new(
        "session",
        AppIcon::MagnifyingGlass,
        "Find session",
        Some("/work/pi".into()),
        None,
        "resume thread",
    );

    assert!(row.matches("find pi"));
    assert!(row.matches("resume"));
    assert!(!row.matches("project settings"));
}

#[gpui::test]
fn picker_rows_render_single_keys_and_key_sequences(cx: &mut gpui::TestAppContext) {
    use gpui::AppContext as _;

    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let (empty, _) = PickerDelegate::new(vec![]);
        let list = cx.new(|cx| ListState::new(empty, window, cx));
        list.update(cx, |_, cx| {
            for shortcut in ["ctrl-g shift-n", "ctrl-shift-p"] {
                let row = PickerRow::new(
                    "action",
                    AppIcon::Code,
                    "Action",
                    None,
                    Some(shortcut.into()),
                    "",
                );
                let (mut delegate, _) = PickerDelegate::new(vec![row]);
                assert!(
                    delegate
                        .render_item(IndexPath::default(), window, cx)
                        .is_some()
                );
            }
        });
    });
}

#[gpui::test]
fn disabled_rows_cannot_be_confirmed_after_search(cx: &mut gpui::TestAppContext) {
    use gpui::AppContext as _;

    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    cx.update(|window, cx| {
        let row = |id, disabled| {
            PickerRow::new(id, AppIcon::Code, id, None, None, "harness").disabled(disabled)
        };
        let (mut delegate, handles) =
            PickerDelegate::new(vec![row("missing", true), row("installed", false)]);
        let (empty, _) = PickerDelegate::new(vec![]);
        let list = cx.new(|cx| ListState::new(empty, window, cx));
        list.update(cx, |_, cx| {
            delegate.perform_search("installed", window, cx).detach();
            delegate.confirm(false, window, cx);
            assert_eq!(handles.confirmed_id.borrow().as_deref(), Some("installed"));
            delegate.perform_search("missing", window, cx).detach();
            delegate.confirm(false, window, cx);
            assert!(handles.confirmed_id.borrow().is_none());
        });
    });
}
