use super::*;

#[gpui::test]
fn plan_dialog_keeps_its_bounds_across_lists_and_details(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::plan_dialog_keeps_its_bounds_across_lists_and_details"
        ),
        cx,
        |cx, app, _, project| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.open_workgraph_surface(window, cx));
            });
            let (board, data, node) = cx.update(|_, cx| {
                let board = app.read(cx).views.workgraph.clone();
                let (data, node) = board
                    .read(cx)
                    .store
                    .as_ref()
                    .unwrap()
                    .with(|store| {
                        store.with_connection(|db| {
                            workgraph::SqliteAdapter::initialize_connection(db).unwrap();
                            workgraph::create_plan(
                                db,
                                project.to_path_buf(),
                                "Review".into(),
                                "Check UI".into(),
                            )
                        })
                    })
                    .unwrap();
                (board, data, node)
            });
            let plan = data.snapshot.as_ref().unwrap().plan.number;
            let mut initial = None;
            for state in 0..4 {
                cx.update(|window, cx| {
                    board.update(cx, |view, cx| {
                        view.state = PlanLoadState::Ready(Box::new(data.clone()));
                        view.all_plans = state == 0 || state == 3;
                        view.selected = (state == 2).then_some(node);
                        view.catalog.selected = (state == 3).then_some(plan);
                        cx.notify();
                    });
                    window.draw(cx).clear(cx);
                });
                let bounds = cx.debug_bounds("project-plan-dialog").expect("plan dialog");
                assert!(f32::from(bounds.size.width) <= super::super::layout::BOARD_WIDTH);
                assert!(
                    f32::from(bounds.size.height)
                        <= f32::from(theme().size(super::super::layout::BOARD_HEIGHT))
                );
                if let Some(initial) = initial {
                    assert_eq!(bounds, initial, "dialog moved or resized in state {state}");
                } else {
                    initial = Some(bounds);
                }
            }
        },
    );
}

#[gpui::test]
fn local_plan_is_ready_before_the_first_frame_and_stays_visible_during_refresh(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_prepared_offline_app(
        concat!(
            module_path!(),
            "::local_plan_is_ready_before_the_first_frame_and_stays_visible_during_refresh"
        ),
        cx,
        |project| {
            crate::app::persistence::open()
                .unwrap()
                .with_connection(|db| {
                    workgraph::SqliteAdapter::initialize_connection(db).unwrap();
                    workgraph::create_plan(
                        db,
                        project.to_path_buf(),
                        "Review".into(),
                        "Check UI".into(),
                    )
                    .unwrap();
                });
        },
        |cx, app, _, _| {
            cx.update(|_, cx| {
                let board = app.read(cx).views.workgraph.clone();
                board.update(cx, |view, cx| {
                    let PlanLoadState::Ready(data) = &view.state else {
                        panic!("local plan must be ready")
                    };
                    assert_eq!(data.plans[0].title, "Review");
                    let before = data.clone();
                    view.refresh(cx);
                    assert_eq!(view.state, PlanLoadState::Ready(before));
                });
            });
        },
    );
}
