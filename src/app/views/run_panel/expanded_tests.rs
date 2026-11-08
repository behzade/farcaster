use gpui::{ParentElement as _, Styled as _};
use std::sync::Arc;

struct PanelHarness(gpui::Entity<crate::app::RunPanelView>, gpui::Pixels);
impl gpui::Render for PanelHarness {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        gpui::div()
            .w(self.1)
            .h(gpui::px(800.0))
            .child(self.0.clone())
    }
}

use crate::{app::FarcasterApp, sessions::SessionSummary};

#[gpui::test]
fn expanded_workers_keep_the_main_row_and_restore_sections_on_navigation(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::expanded_workers_keep_the_main_row_and_restore_sections_on_navigation"
        ),
        cx,
        |cx, app, _, project| {
            let project_path = project.canonicalize().unwrap();
            let project = project_path.as_path();
            crate::app::project::trust::apply(project, crate::projects::TrustChoice::TrustProject)
                .unwrap();
            assert!(
                std::process::Command::new("git")
                    .args(["init", "-q"])
                    .current_dir(project)
                    .status()
                    .unwrap()
                    .success()
            );
            let root = project.join("root.jsonl");
            let other = project.join("other.jsonl");
            let sessions = [
                ("root", None),
                ("other", None),
                ("a", Some("root")),
                ("b", Some("root")),
                ("c", Some("root")),
                ("d", Some("root")),
            ]
            .map(|(id, parent)| {
                SessionSummary::from_cached(
                    id.into(),
                    project.join(format!("{id}.jsonl")),
                    project.into(),
                    id.into(),
                    String::new(),
                    String::new(),
                    parent.map(str::to_owned),
                    std::time::SystemTime::now(),
                    0,
                    Default::default(),
                    false,
                    false,
                    String::new(),
                )
            });
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    for session in &sessions {
                        std::fs::write(&session.path, "{}").unwrap();
                        app.activity.row_focus.insert(
                            crate::agent_activity::agent_activity_key(&session.path),
                            cx.focus_handle(),
                        );
                    }
                    app.project.path = project.into();
                    app.project.repository.project = project.into();
                    Arc::make_mut(&mut app.snapshot).project = project.into();
                    app.sessions.all = sessions.to_vec().into();
                    app.sessions.visible = sessions.to_vec().into();
                    app.lifecycle.pending_session_switch = None;
                    app.sessions.selected_draft = None;
                    Arc::make_mut(&mut app.snapshot).selected_session = Some(root.clone());
                    app.project.repository.backend =
                        crate::repository::RepositoryBackend::discover(
                            project,
                            crate::repository::BackendPreference::Git,
                        )
                        .unwrap();
                    app.views.run_panel.update(cx, |_, cx| cx.notify());
                    cx.notify();
                })
            });
            cx.update(|window, cx| {
                let panel = app.read(cx).views.run_panel.clone();
                window.replace_root(cx, |_, _| PanelHarness(panel, gpui::px(332.0)));
            });
            let draw = |cx: &mut gpui::VisualTestContext| {
                cx.update(|window, cx| {
                    window.draw(cx).clear(cx);
                })
            };
            draw(cx);
            cx.update(|_, cx| {
                let app = app.read(cx);
                assert_eq!(app.snapshot.selected_session.as_ref(), Some(&root));
                assert!(app.sessions.all.root_for_path(Some(&root)).is_some());
            });
            let main_before = cx.debug_bounds("run-panel-main-agent").unwrap();
            assert!(cx.debug_bounds("run-panel-repository").is_some());
            let show = cx.debug_bounds("show-more-workers").unwrap();
            cx.simulate_click(show.center(), Default::default());
            draw(cx);
            assert_eq!(
                cx.debug_bounds("run-panel-main-agent").unwrap(),
                main_before
            );
            assert!(cx.debug_bounds("show-more-workers").is_none());
            assert!(cx.debug_bounds("run-panel-plan").is_none());
            assert!(cx.debug_bounds("run-panel-repository").is_none());
            cx.simulate_click(main_before.center(), Default::default());
            draw(cx);
            assert!(cx.debug_bounds("run-panel-repository").is_some());
            let show = cx.debug_bounds("show-more-workers").unwrap();
            cx.simulate_click(show.center(), Default::default());
            draw(cx);
            cx.update(|_, cx| {
                assert!(
                    app.read(cx)
                        .views
                        .run_panel
                        .read(cx)
                        .workers_expanded_for(&root)
                )
            });
            let child_selector = Box::leak(
                format!("agent-card-{}", project.join("a.jsonl").display()).into_boxed_str(),
            );
            let child = cx
                .debug_bounds(child_selector)
                .expect("oldest worker visible");
            cx.simulate_click(child.center(), Default::default());
            draw(cx);
            cx.update(|_, cx| {
                assert_eq!(
                    app.read(cx)
                        .lifecycle
                        .pending_session_switch
                        .as_ref()
                        .map(|(path, _)| path),
                    Some(&project.join("a.jsonl"))
                );
                assert!(
                    app.read(cx)
                        .views
                        .run_panel
                        .read(cx)
                        .workers_expanded_for(&root)
                )
            });
            let main = cx.debug_bounds("run-panel-main-agent").unwrap();
            cx.simulate_click(main.center(), Default::default());
            draw(cx);
            cx.update(|_, cx| {
                assert_eq!(
                    app.read(cx)
                        .lifecycle
                        .pending_session_switch
                        .as_ref()
                        .map(|(path, _)| path),
                    Some(&root)
                );
                assert!(
                    !app.read(cx)
                        .views
                        .run_panel
                        .read(cx)
                        .workers_expanded_for(&root)
                )
            });
            assert!(cx.debug_bounds("run-panel-repository").is_some());
            assert!(cx.debug_bounds("show-more-workers").is_some());

            for draft in [false, true] {
                cx.update(|_, cx| {
                    app.update(cx, |app: &mut FarcasterApp, cx| {
                        app.lifecycle.pending_session_switch = None;
                        app.sessions.selected_draft = None;
                        Arc::make_mut(&mut app.snapshot).selected_session = Some(root.clone());
                        app.views.run_panel.update(cx, |_, cx| cx.notify());
                    })
                });
                draw(cx);
                let show = cx.debug_bounds("show-more-workers").unwrap();
                cx.simulate_click(show.center(), Default::default());
                draw(cx);
                cx.update(|_, cx| {
                    app.update(cx, |app, cx| {
                        if draft {
                            app.sessions.selected_draft = Some("new-draft".into());
                        } else {
                            app.lifecycle.pending_session_switch = Some((
                                other.clone(),
                                crate::app::infrastructure::performance::Timing::new("test.switch"),
                            ));
                        }
                        app.views.run_panel.update(cx, |_, cx| cx.notify());
                    })
                });
                draw(cx);
                cx.update(|_, cx| {
                    assert!(
                        !app.read(cx)
                            .views
                            .run_panel
                            .read(cx)
                            .workers_expanded_for(&root)
                    )
                });
                assert!(cx.debug_bounds("run-panel-repository").is_some());
            }
        },
    );
}

#[gpui::test]
fn expanded_workers_build_only_visible_cards_and_reveal_selection(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::expanded_workers_build_only_visible_cards_and_reveal_selection"
        ),
        cx,
        |cx, app, _, project| {
            let project_path = project.canonicalize().unwrap();
            let project = project_path.as_path();
            crate::app::project::trust::apply(project, crate::projects::TrustChoice::TrustProject)
                .unwrap();
            let root = project.join("root.jsonl");
            let oldest = project.join("worker-0000.jsonl");
            let sessions: Vec<_> = std::iter::once(("root".to_owned(), None, 0))
                .chain((0..500).map(|i| (format!("worker-{i:04}"), Some("root".to_owned()), i)))
                .map(|(id, parent, age)| {
                    SessionSummary::from_cached(
                        id.clone(),
                        project.join(format!("{id}.jsonl")),
                        project.into(),
                        id,
                        String::new(),
                        String::new(),
                        parent,
                        std::time::UNIX_EPOCH + std::time::Duration::from_secs(age),
                        0,
                        Default::default(),
                        false,
                        false,
                        String::new(),
                    )
                })
                .collect();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    for session in &sessions {
                        app.activity.row_focus.insert(
                            crate::agent_activity::agent_activity_key(&session.path),
                            cx.focus_handle(),
                        );
                    }
                    std::fs::write(&oldest, "{}").unwrap();
                    app.sessions.visible = sessions.clone().into();
                    app.sessions.all = sessions.into();
                    app.lifecycle.pending_session_switch = None;
                    app.sessions.selected_draft = None;
                    Arc::make_mut(&mut app.snapshot).selected_session = Some(root.clone());
                    app.views.run_panel.update(cx, |_, cx| cx.notify());
                });
                let panel = app.read(cx).views.run_panel.clone();
                window.replace_root(cx, |_, _| PanelHarness(panel, gpui::px(332.0)));
            });
            let draw = |cx: &mut gpui::VisualTestContext| {
                super::agents::CARD_BUILDS.with(|count| count.set(0));
                cx.update(|window, cx| {
                    window.draw(cx).clear(cx);
                });
                let built = super::agents::CARD_BUILDS.with(|count| count.get());
                assert!(
                    built < 60,
                    "built {built} cards for a viewport of 500 workers"
                );
            };
            draw(cx);
            let show = cx.debug_bounds("show-more-workers").unwrap();
            super::agents::CARD_BUILDS.with(|count| count.set(0));
            cx.simulate_click(show.center(), Default::default());
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });
            let built = super::agents::CARD_BUILDS.with(|count| count.get());
            assert!(built > 0 && built < 150, "expanding built {built} cards");
            assert!(cx.debug_bounds("show-more-workers").is_none());
            assert!(cx.debug_bounds("run-panel-main-agent").is_some());
            let oldest_selector = Box::leak(
                format!(
                    "agent-card-{}",
                    crate::agent_activity::agent_activity_key(&oldest)
                )
                .into_boxed_str(),
            );
            assert!(cx.debug_bounds(oldest_selector).is_none());

            // This row stays visible before and after the 600px wheel movement.
            let distance_selector = Box::leak(
                format!(
                    "agent-card-{}",
                    crate::agent_activity::agent_activity_key(&project.join("worker-0487.jsonl"))
                )
                .into_boxed_str(),
            );
            let before_scroll = cx.debug_bounds(distance_selector).unwrap();
            super::worker_list::PREPARATIONS.with(|count| count.set(0));
            // Exercise the actual wheel path, not just programmatic selection.
            super::agents::CARD_BUILDS.with(|count| count.set(0));
            let pointer = cx.debug_bounds("run-panel-main-agent").unwrap().center();
            cx.simulate_event(gpui::ScrollEvent {
                position: pointer,
                delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.0), gpui::px(-600.0))),
                modifiers: Default::default(),
                touch_phase: gpui::TouchPhase::Moved,
                source: gpui::ScrollSource::Pointer,
            });
            cx.update(|window, cx| {
                window.draw(cx).clear(cx);
            });
            let built = super::agents::CARD_BUILDS.with(|count| count.get());
            assert!(built < 100, "wheel scroll built {built} cards");
            assert_eq!(
                super::worker_list::PREPARATIONS.with(|count| count.get()),
                0,
                "wheel input must reuse preparation without filesystem work for all workers"
            );
            assert!(cx.debug_bounds("run-panel-main-agent").is_none());
            let after_scroll = cx.debug_bounds(distance_selector).unwrap();
            assert!(
                (before_scroll.top() - after_scroll.top() - gpui::px(600.0)).abs() < gpui::px(1.0),
                "wheel distance must include offscreen rows"
            );

            // Keyboard navigation can select a worker far outside the current viewport.
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    Arc::make_mut(&mut app.snapshot).selected_session = Some(oldest.clone());
                    app.views.run_panel.update(cx, |_, cx| cx.notify());
                });
            });
            draw(cx);
            assert!(
                cx.debug_bounds(oldest_selector).is_some(),
                "selected worker must scroll into view"
            );
            assert!(cx.debug_bounds("run-panel-main-agent").is_none());

            assert_eq!(
                super::worker_list::PREPARATIONS.with(|count| count.get()),
                0,
                "selection in the same family must reuse prepared workers"
            );
            cx.update(|_, cx| {
                app.update(cx, |app, cx| app.notify_run_panel(cx));
            });
            draw(cx);
            assert_eq!(
                super::worker_list::PREPARATIONS.with(|count| count.get()),
                1,
                "application updates must invalidate the worker snapshot"
            );

            // An unrelated panel redraw must retain the scroll position.
            cx.update(|_, cx| {
                let panel = app.read(cx).views.run_panel.clone();
                panel.update(cx, |_, cx| cx.notify());
            });
            draw(cx);
            assert!(cx.debug_bounds(oldest_selector).is_some());

            let scroll_to_top = |cx: &mut gpui::VisualTestContext| {
                cx.simulate_event(gpui::ScrollEvent {
                    position: gpui::point(gpui::px(150.0), gpui::px(200.0)),
                    delta: gpui::ScrollDelta::Pixels(gpui::point(
                        gpui::px(0.0),
                        gpui::px(100_000.0),
                    )),
                    ..Default::default()
                });
                draw(cx);
            };
            scroll_to_top(cx);
            assert!(
                cx.debug_bounds("run-panel-main-agent").is_some(),
                "wheel must return to the top before selecting across the gap"
            );
            // A measured tail must not make wheel scrolling skip the unseen middle.
            cx.simulate_event(gpui::ScrollEvent {
                position: gpui::point(gpui::px(150.0), gpui::px(200.0)),
                delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.0), gpui::px(-1500.0))),
                ..Default::default()
            });
            draw(cx);
            let middle_selector = Box::leak(
                format!(
                    "agent-card-{}",
                    crate::agent_activity::agent_activity_key(&project.join("worker-0468.jsonl"))
                )
                .into_boxed_str(),
            );
            assert!(
                cx.debug_bounds(middle_selector).is_some(),
                "wheel must traverse the middle rows"
            );
            assert!(cx.debug_bounds(oldest_selector).is_none());
            scroll_to_top(cx);

            // Resizing must preserve full-list wheel geometry too.
            cx.update(|window, cx| {
                let panel = app.read(cx).views.run_panel.clone();
                window.replace_root(cx, |_, _| PanelHarness(panel, gpui::px(420.0)));
            });
            draw(cx);
            let before_resize_scroll = cx.debug_bounds(distance_selector).unwrap();
            cx.simulate_event(gpui::ScrollEvent {
                position: gpui::point(gpui::px(150.0), gpui::px(200.0)),
                delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.0), gpui::px(-600.0))),
                ..Default::default()
            });
            draw(cx);
            let after_resize_scroll = cx.debug_bounds(distance_selector).unwrap();
            assert!(
                (before_resize_scroll.top() - after_resize_scroll.top() - gpui::px(600.0)).abs()
                    < gpui::px(1.0)
            );
            scroll_to_top(cx);
            let next_oldest = project.join("worker-0020.jsonl");
            let next_selector = Box::leak(
                format!(
                    "agent-card-{}",
                    crate::agent_activity::agent_activity_key(&next_oldest)
                )
                .into_boxed_str(),
            );
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    Arc::make_mut(&mut app.snapshot).selected_session = Some(next_oldest.clone());
                    app.views.run_panel.update(cx, |_, cx| cx.notify());
                });
            });
            draw(cx);
            assert!(
                cx.debug_bounds(next_selector).is_some(),
                "measured worker across an unmeasured gap must be revealed"
            );

            cx.update(|window, cx| {
                let focus = app.read(cx).activity.row_focus
                    [&crate::agent_activity::agent_activity_key(&oldest)]
                    .clone();
                focus.focus(window, cx);
            });
            scroll_to_top(cx);
            cx.update(|window, cx| {
                assert!(
                    app.read(cx).activity.row_focus
                        [&crate::agent_activity::agent_activity_key(&oldest)]
                        .is_focused(window)
                );
            });
            cx.simulate_keystrokes("enter");
            cx.update(|_, cx| {
                assert_eq!(
                    app.read(cx)
                        .lifecycle
                        .pending_session_switch
                        .as_ref()
                        .map(|(path, _)| path),
                    Some(&oldest),
                    "offscreen focused worker must retain its Enter handler"
                );
            });
        },
    );
}
