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
        let mut restored = child.pop_previous().unwrap();
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
            restored.pop_previous().unwrap().scope,
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
