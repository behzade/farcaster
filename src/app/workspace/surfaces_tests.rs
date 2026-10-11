use super::*;

#[gpui::test]
fn worker_notices_block_surface_switch_until_closed(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::worker_notices_block_surface_switch_until_closed"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    for surface in [AppSurface::Editor, AppSurface::Terminal] {
                        app.set_surface(surface, cx);
                        app.open_worker_notices(window, cx);
                        assert!(app.native_workspace_modal_active());
                        assert_eq!(
                            app.native_surface_obscured(window, cx),
                            !cfg!(target_os = "macos")
                        );
                        app.cycle_workspace_surface(true, window, cx);
                        assert_eq!(app.workspace.surface, surface);

                        app.close_sheet(window, cx);
                        assert!(!app.native_surface_obscured(window, cx));
                        assert!(!app.workspace.native_surface_covered);
                        assert_eq!(app.workspace.surface, surface);
                    }
                });
            });
        },
    );
}

#[gpui::test]
fn move_and_model_access_dialogs_block_workspace_surface_cycle(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::move_and_model_access_dialogs_block_workspace_surface_cycle"
        ),
        cx,
        |cx, app, _, project| {
            let source = project.join("running-session.jsonl");
            let session = crate::sessions::SessionSummary::from_cached(
                "running-session".into(),
                source.clone(),
                project.to_path_buf(),
                "Running session".into(),
                String::new(),
                String::new(),
                None,
                std::time::SystemTime::now(),
                0,
                Default::default(),
                false,
                true,
                String::new(),
            );
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.sessions.all.push(session);
                    app.set_surface(AppSurface::Terminal, cx);
                    app.move_session(source, project.join("target"), window, cx);
                    assert!(app.sessions.pending_move.is_some());
                    app.cycle_workspace_surface(true, window, cx);
                    assert_eq!(app.workspace.surface, AppSurface::Terminal);

                    app.close_move_confirmation(window, cx);
                    app.navigation.pending_model_access =
                        Some(crate::app::navigation::PendingModelAccess {
                            focus: cx.focus_handle(),
                            model: serde_json::from_value(serde_json::json!({
                                "id": "test-model", "name": "Test model", "provider": "test"
                            }))
                            .expect("model fixture"),
                            modes: Vec::new(),
                            effort: None,
                            apply_effort: false,
                            return_focus: None,
                        });
                    app.cycle_workspace_surface(true, window, cx);
                    assert_eq!(app.workspace.surface, AppSurface::Terminal);

                    app.close_model_access_confirmation(window, cx);
                    app.cycle_workspace_surface(true, window, cx);
                    assert_eq!(app.workspace.surface, AppSurface::Chat);
                });
            });
        },
    );
}

#[gpui::test]
fn workspace_bar_hover_uses_platform_overlay_policy(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::workspace_bar_hover_uses_platform_overlay_policy"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    assert!(!app.native_surface_obscured(window, cx));
                    app.set_workspace_bar_hovered(true, cx);
                    assert_eq!(
                        app.native_surface_obscured(window, cx),
                        !cfg!(target_os = "macos")
                    );
                    app.set_workspace_bar_hovered(false, cx);
                    assert!(!app.native_surface_obscured(window, cx));
                });
            });
        },
    );
}

#[gpui::test]
fn native_surface_focus_waits_only_for_the_snapshot_fallback(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::native_surface_focus_waits_only_for_the_snapshot_fallback"
        ),
        cx,
        |cx, app, _, _| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.set_surface(AppSurface::Terminal, cx);
                    app.set_workspace_bar_hovered(true, cx);
                    app.request_active_surface_focus(None);
                    app.apply_post_render_focus(window, cx);
                    assert_eq!(
                        app.overlays.post_render_focus.is_some(),
                        !cfg!(target_os = "macos")
                    );
                    app.set_workspace_bar_hovered(false, cx);
                    app.apply_post_render_focus(window, cx);
                    assert!(app.overlays.post_render_focus.is_none());
                });
            });
        },
    );
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
