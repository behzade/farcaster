use std::sync::Arc;

use gpui::{px, size};

#[gpui::test]
fn toolbar_keeps_room_for_session_identity_without_overlapping_navigation(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::toolbar_keeps_room_for_session_identity_without_overlapping_navigation"
        ),
        cx,
        |cx, app, _, project| {
            let session = crate::sessions::SessionSummary::from_cached(
                "toolbar-session".into(),
                project.join("toolbar-session.jsonl"),
                project.into(),
                "Blind Multi-Worker UI Design Comparison".into(),
                String::new(),
                String::new(),
                None,
                std::time::SystemTime::now(),
                0,
                Default::default(),
                false,
                false,
                String::new(),
            );
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    Arc::make_mut(&mut app.snapshot).selected_session = Some(session.path.clone());
                    app.sessions.all = vec![session.clone()].into();
                    app.sessions.visible = vec![session].into();
                    app.sessions.selected_draft = None;
                    app.lifecycle.pending_session_switch = None;
                    cx.notify();
                });
            });

            for width in [1240.0, 1600.0, 1000.0, 800.0] {
                cx.simulate_resize(size(px(width), px(820.0)));
                cx.update(|window, cx| window.draw(cx).clear(cx));
                let bar = cx.debug_bounds("workspace-bar").unwrap();
                let identity = cx.debug_bounds("workspace-breadcrumb").unwrap();
                let title = cx.debug_bounds("workspace-session-title").unwrap();
                let navigation = cx.debug_bounds("workspace-navigation").unwrap();
                assert!(identity.right() < navigation.left(), "width {width}");
                assert!(navigation.right() <= bar.right(), "width {width}");
                assert!(
                    identity.size.width >= bar.size.width * 0.35,
                    "session identity needs a meaningful share at width {width}: {identity:?} {bar:?}"
                );
                assert!(title.size.width >= px(100.0), "width {width}: {title:?}");

                let mut previous = None;
                for id in [
                    "show-chat-surface",
                    "show-editor-surface",
                    "show-terminal-surface",
                ] {
                    let bounds = cx.debug_bounds(id).unwrap();
                    assert_eq!(bounds.top(), bar.top(), "{id} at width {width}");
                    assert_eq!(
                        bounds.size.height,
                        bar.size.height - crate::app::ui::theme::theme().border
                    );
                    assert!(bounds.size.width >= px(28.0));
                    if let Some(right) = previous {
                        assert!(bounds.left() >= right, "{id} at width {width}");
                    }
                    previous = Some(bounds.right());
                }
            }

            cx.simulate_resize(size(px(1240.0), px(820.0)));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let settings = cx.debug_bounds("sidebar-settings-control").unwrap();
            let plan = cx.debug_bounds("sidebar-plan-control").unwrap();
            let notices = cx.debug_bounds("sidebar-notices-control").unwrap();
            for bounds in [plan, notices] {
                assert_eq!(bounds.top(), settings.top());
                assert_eq!(bounds.size, settings.size);
            }
            assert!(plan.left() > settings.right());
            assert!(notices.left() > plan.right());
            cx.simulate_click(plan.center(), Default::default());
            cx.run_until_parked();
            cx.update(|_, cx| {
                assert_eq!(app.read(cx).workspace.surface, crate::app::AppSurface::Work);
            });
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.show_chat_surface(window, cx));
                window.draw(cx).clear(cx);
            });
            let notices = cx.debug_bounds("sidebar-notices-control").unwrap();
            cx.simulate_click(notices.center(), Default::default());
            cx.run_until_parked();
            cx.update(|_, cx| {
                assert!(app.read(cx).overlays.view.worker_notices);
            });

            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.close_sheet(window, cx));
            });
            cx.simulate_resize(size(px(800.0), px(820.0)));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.open_sessions_sheet(window, cx));
                window.draw(cx).clear(cx);
            });
            let sheet_plan = cx.debug_bounds("sidebar-plan-control").unwrap();
            cx.simulate_click(sheet_plan.center(), Default::default());
            cx.run_until_parked();
            cx.update(|_, cx| {
                let app = app.read(cx);
                assert_eq!(app.workspace.surface, crate::app::AppSurface::Work);
                assert!(
                    !app.overlays.view.sessions,
                    "the Sessions sheet must not cover the plan opened from its footer"
                );
            });
        },
    );
}
