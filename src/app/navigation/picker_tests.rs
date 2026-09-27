use super::*;

#[gpui::test]
fn back_restores_search_selection_scroll_and_parent_history(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let cx = cx.add_empty_window();
    let make_page = |scope, window: &mut Window, cx: &mut gpui::App| {
        let rows = (0..30)
            .map(|index| {
                PickerRow::new(
                    format!("row:{index}"),
                    AppIcon::List,
                    format!("Model {index}"),
                    None,
                    None,
                    "",
                )
            })
            .collect();
        let (delegate, handles) = PickerDelegate::new(rows);
        let list = cx.new(|cx| ComponentListState::new(delegate, window, cx).searchable(true));
        let subscription = cx.subscribe(&list, |_, _: &ListEvent, _| {});
        PickerState {
            scope,
            list,
            commands: HashMap::new(),
            query: handles.query,
            _subscription: subscription,
            previous: None,
        }
    };
    let mut page =
        cx.update(|window, cx| make_page(PickerScope::Models("provider".into()), window, cx));
    cx.update(|window, cx| {
        page.list
            .update(cx, |list, cx| list.set_query("Model", window, cx))
    });
    cx.run_until_parked();
    let selected = IndexPath {
        row: 17,
        ..Default::default()
    };
    let offset = gpui::point(gpui::px(0.0), gpui::px(-280.0));
    cx.update(|window, cx| {
        page.list.update(cx, |list, cx| {
            list.set_selected_index(Some(selected), window, cx);
            list.scroll_handle().base_handle().set_offset(offset);
        });
        page.previous = Some(Box::new(make_page(PickerScope::Providers, window, cx)));
        let list_id = page.list.entity_id();
        let mut child = make_page(PickerScope::Sandbox, window, cx);
        child.previous = Some(Box::new(page));
        assert!(child.has_ancestor(&PickerScope::Providers));
        assert!(!child.has_ancestor(&PickerScope::Actions));
        let mut restored = child.pop_previous().expect("test operation should succeed");
        assert_eq!(restored.list.entity_id(), list_id);
        assert_eq!(&*restored.query.borrow(), "Model");
        assert_eq!(restored.list.read(cx).selected_index(), Some(selected));
        assert_eq!(
            restored
                .list
                .read(cx)
                .scroll_handle()
                .base_handle()
                .offset(),
            offset
        );
        assert_eq!(
            restored
                .pop_previous()
                .expect("test operation should succeed")
                .scope,
            PickerScope::Providers
        );
        assert!(restored.pop_previous().is_none());
    });
}

#[test]
fn move_project_choices_exclude_the_source_project() {
    let source = PathBuf::from("/work/source");
    let target = PathBuf::from("/work/target");
    let intent = ProjectPickerIntent::MoveSession {
        path: PathBuf::from("/sessions/session.jsonl"),
        source_project: source.clone(),
    };

    assert!(!project_is_available_for_intent(&intent, &source));
    assert!(project_is_available_for_intent(&intent, &target));
}

#[test]
fn projects_with_recent_sessions_lead_then_registry_order_breaks_ties() {
    let alpha = PathBuf::from("/work/alpha");
    let beta = PathBuf::from("/work/beta");
    let gamma = PathBuf::from("/work/gamma");
    let recency = HashMap::from([
        (alpha.clone(), Duration::from_secs(10)),
        (beta.clone(), Duration::from_secs(20)),
    ]);

    assert_eq!(
        sort_projects_by_recency(&[alpha.clone(), gamma.clone(), beta.clone()], &recency,),
        vec![beta, alpha, gamma]
    );
}

#[test]
fn open_session_project_leads_without_changing_the_remaining_order() {
    let alpha = PathBuf::from("/work/alpha");
    let beta = PathBuf::from("/work/beta");
    let gamma = PathBuf::from("/work/gamma");

    assert_eq!(
        ordered_projects(
            &[alpha.clone(), beta.clone(), gamma.clone()],
            &[],
            Some(&gamma),
        ),
        vec![gamma, alpha, beta]
    );
}

#[gpui::test]
fn action_picker_routes_themes_and_models_without_losing_parent_context(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::action_picker_routes_themes_and_models_without_losing_parent_context"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.settings.tab = crate::app::workspace::SettingsTab::Workers;
                    app.open_picker(PickerScope::Actions, window, cx);
                    let (rows, commands) = app.action_picker_rows();
                    assert_eq!(rows[0].id, "action:new-session");
                    assert!(rows.iter().all(|row| !row.section.is_empty()));
                    assert_eq!(
                        commands.get("action:themes"),
                        Some(&PickerCommand::OpenThemes)
                    );
                    app.execute_picker_row("action:settings", window, cx);
                    assert!(app.settings.tab == crate::app::workspace::SettingsTab::Workers);
                    app.open_picker(PickerScope::Actions, window, cx);
                    app.settings.themes.editing = true;
                    app.execute_picker_row("action:themes", window, cx);
                    assert!(app.settings.tab == crate::app::workspace::SettingsTab::Appearance);
                    assert!(!app.settings.themes.editing);

                    let model: crate::protocol::Model = serde_json::from_value(serde_json::json!({
                    "id": "test", "name": "Test model", "provider": "provider", "reasoning": true,
                    "efforts": ["low", "high"]
                })).unwrap();
                    let snapshot = std::sync::Arc::make_mut(&mut app.snapshot);
                    snapshot.harness = Some(Backend::Codex);
                    snapshot.prefill_model = Some(model.clone());
                    snapshot.prefill_thinking_level = Some("high".into());
                    snapshot.models = vec![model];
                    app.open_picker(PickerScope::Actions, window, cx);
                    let actions = app.navigation.picker.as_ref().unwrap().list.clone();
                    actions.update(cx, |list, cx| list.set_query("model", window, cx));
                    app.open_runtime_picker(window, cx);
                    let page = app.navigation.picker.as_ref().unwrap();
                    assert_eq!(page.scope, PickerScope::Models("provider".into()));
                    assert_eq!(
                        page.list.read(cx).selected_index().map(|index| index.row),
                        Some(0)
                    );
                    assert!(page.has_ancestor(&PickerScope::Actions));
                    assert!(page.has_ancestor(&PickerScope::Providers));
                    app.picker_navigate_back(window, cx);
                    app.picker_navigate_back(window, cx);
                    assert_eq!(
                        app.navigation.picker.as_ref().unwrap().list.entity_id(),
                        actions.entity_id()
                    );
                    assert_eq!(
                        &*app.navigation.picker.as_ref().unwrap().query.borrow(),
                        "model"
                    );
                })
            });
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.simulate_keystrokes("escape");
            cx.update(|_, cx| assert!(app.read(cx).navigation.picker.is_none()));
        },
    );
}
