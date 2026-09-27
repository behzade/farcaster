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
#[test]
fn replacing_picker_rows_keeps_search_and_selected_row() {
    let row = |id: &str, label: &str| PickerRow::new(id, AppIcon::List, label, None, None, "");
    let (mut picker, handles) =
        PickerDelegate::new(vec![row("one", "First"), row("two", "Second")]);
    *handles.query.borrow_mut() = "sec".into();
    picker.replace_rows(vec![row("one", "First"), row("two", "Second")]);
    assert_eq!(picker.visible_rows.len(), 1);
    assert_eq!(picker.visible_rows[0].id, "two");
    assert_eq!(picker.selected_index.map(|index| index.row), Some(0));

    picker.replace_rows(vec![row("three", "Second choice")]);
    assert_eq!(picker.visible_rows[0].id, "three");
    assert_eq!(picker.selected_index.map(|index| index.row), Some(0));
}

#[test]
fn search_ranks_labels_then_keywords_and_keeps_disabled_matches_last() {
    let row = |id, label, disabled| {
        PickerRow::new(id, AppIcon::Code, label, None, None, "model")
            .disabled(disabled)
            .section("Configure")
    };
    let (mut picker, _) = PickerDelegate::new(vec![
        row("disabled", "Model unavailable", true),
        row("keyword", "Settings", false),
        row("label", "Choose model", false),
        row("label-two", "Model effort", false),
    ]);
    picker.filter_rows("MODEL");
    assert_eq!(
        picker
            .visible_rows
            .iter()
            .map(|row| row.id.as_str())
            .collect::<Vec<_>>(),
        ["label", "label-two", "keyword", "disabled"]
    );
    assert_eq!(picker.sections.len(), 1);
    assert_eq!(
        picker.row(picker.selected_index.unwrap()).unwrap().id,
        "label"
    );
    picker.filter_rows("unavailable");
    assert_eq!(picker.visible_rows.len(), 1);
    assert_eq!(picker.selected_index, None);
}

#[gpui::test]
fn keyboard_skips_disabled_rows_across_sections_and_after_search(cx: &mut gpui::TestAppContext) {
    use gpui::Focusable as _;
    cx.update(gpui_component::init);
    let row = |id, section, disabled| {
        PickerRow::new(id, AppIcon::Code, id, None, None, "")
            .section(section)
            .disabled(disabled)
    };
    let (delegate, handles) = PickerDelegate::new(vec![
        row("disabled-first", "Session", true),
        row("session", "Session", false),
        row("disabled-middle", "Configure", true),
        row("model", "Configure", false),
        row("disabled-last", "Application", true),
    ]);
    let first = delegate.preferred_index(None);
    let (list, cx) =
        cx.add_window_view(|window, cx| ListState::new(delegate, window, cx).searchable(true));
    cx.update(|window, cx| {
        list.update(cx, |list, cx| {
            list.set_selected_index(first, window, cx);
            list.focus_handle(cx).focus(window, cx);
        });
        window.draw(cx).clear(cx);
    });
    for (key, id) in [("down", "model"), ("down", "session"), ("up", "model")] {
        cx.simulate_keystrokes(key);
        cx.update(|_, cx| {
            let list = list.read(cx);
            assert_eq!(
                list.delegate()
                    .row(list.selected_index().unwrap())
                    .unwrap()
                    .id,
                id
            );
        });
    }
    cx.simulate_keystrokes("enter");
    assert_eq!(handles.confirmed_id.borrow().as_deref(), Some("model"));
    for (query, expected) in [
        ("disabled", None),
        ("model", Some("model")),
        ("missing", None),
    ] {
        cx.update(|window, cx| list.update(cx, |list, cx| list.set_query(query, window, cx)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            let list = list.read(cx);
            assert_eq!(
                list.selected_index()
                    .and_then(|ix| list.delegate().row(ix))
                    .map(|row| row.id.as_str()),
                expected
            );
        });
        *handles.confirmed_id.borrow_mut() = None;
        cx.simulate_keystrokes("down enter");
        assert_eq!(handles.confirmed_id.borrow().as_deref(), expected);
    }
}

#[test]
fn grouped_refresh_preserves_identity_and_replaces_a_disabled_selection() {
    let row = |id, section, disabled| {
        PickerRow::new(id, AppIcon::Code, id, None, None, "")
            .section(section)
            .disabled(disabled)
    };
    let (mut picker, _) = PickerDelegate::new(vec![
        row("session", "Session", false),
        row("model", "Configure", false),
    ]);
    picker.selected_index = picker.preferred_index(Some(1));
    assert_eq!(picker.selected_index.unwrap().section, 1);
    picker.replace_rows(vec![
        row("model", "Configure", false),
        row("session", "Session", false),
    ]);
    assert_eq!(picker.selected_index.unwrap().section, 0);
    assert_eq!(
        picker.row(picker.selected_index.unwrap()).unwrap().id,
        "model"
    );
    picker.replace_rows(vec![
        row("model", "Configure", true),
        row("session", "Session", false),
    ]);
    assert_eq!(
        picker.row(picker.selected_index.unwrap()).unwrap().id,
        "session"
    );
}
