use gpui::{ParentElement as _, Styled as _};
use std::sync::Arc;

struct PanelHarness(gpui::Entity<crate::app::RunPanelView>);
impl gpui::Render for PanelHarness {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        gpui::div()
            .w(gpui::px(332.0))
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
            // with_offline_app runs this closure in a child with a temporary home and data dir.
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
                window.replace_root(cx, |_, _| PanelHarness(panel));
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
            // Clicking an already-selected main agent must also restore the panel.
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
            // The oldest child is now included, alongside the recent workers.
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
