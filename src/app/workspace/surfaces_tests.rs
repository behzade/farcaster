use super::*;

#[gpui::test]
fn hovering_the_workspace_bar_obscures_the_native_surface(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::hovering_the_workspace_bar_obscures_the_native_surface"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    assert!(!app.native_surface_obscured(window, cx));
                    app.set_workspace_bar_hovered(true, cx);
                    assert!(app.native_surface_obscured(window, cx));
                    app.set_workspace_bar_hovered(false, cx);
                    assert!(!app.native_surface_obscured(window, cx));
                });
            });
        },
    );
}

#[gpui::test]
fn focusing_a_native_surface_waits_until_the_bar_stops_obscuring_it(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::focusing_a_native_surface_waits_until_the_bar_stops_obscuring_it"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.set_surface(AppSurface::Terminal, cx);
                    app.set_workspace_bar_hovered(true, cx);
                    app.request_active_surface_focus(None);
                    app.apply_post_render_focus(window, cx);
                    assert!(app.overlays.post_render_focus.is_some());
                    app.set_workspace_bar_hovered(false, cx);
                    app.apply_post_render_focus(window, cx);
                    assert!(app.overlays.post_render_focus.is_none());
                });
            });
        },
    );
}

#[test]
fn arriving_requests_only_focus_when_replacing_the_composer_slot() {
    assert!(arriving_request_takes_focus(true));
    assert!(!arriving_request_takes_focus(false));
}

#[test]
fn activating_a_sheet_never_stacks_it_with_an_existing_sheet() {
    for sheet in [
        AppSheet::Sessions,
        AppSheet::Run,
        AppSheet::WorkerNotices,
        AppSheet::Keybindings,
        AppSheet::Settings,
        AppSheet::ProjectTrust,
    ] {
        let flags = sheet_flags(Some(sheet));
        assert_eq!(
            [
                flags.sessions,
                flags.run,
                flags.worker_notices,
                flags.keybindings,
                flags.settings,
                flags.project_trust,
            ]
            .into_iter()
            .filter(|active| *active)
            .count(),
            1
        );
    }
    assert!(!sheet_flags(None).any());
}

#[test]
fn an_existing_sheet_prevents_recapturing_the_return_focus() {
    assert!(should_capture_return_focus(sheet_flags(None)));
    assert!(!should_capture_return_focus(sheet_flags(Some(
        AppSheet::Sessions
    ))));
    assert!(!should_capture_return_focus(sheet_flags(Some(
        AppSheet::Run
    ))));
}
