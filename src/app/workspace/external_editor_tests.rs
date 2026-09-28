use super::*;
use std::{cell::Cell, rc::Rc};

#[gpui::test]
fn diff_preparation_defers_open_and_checks_project_and_trust(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_prepared_offline_app(
        concat!(
            module_path!(),
            "::diff_preparation_defers_open_and_checks_project_and_trust"
        ),
        cx,
        |project| {
            assert!(
                Command::new("git")
                    .args(["init", "-q"])
                    .arg(project)
                    .status()
                    .unwrap()
                    .success()
            );
            std::fs::write(project.join("file.rs"), "new file\n").unwrap();
        },
        |cx, app, _, project| {
            for scenario in [
                "current",
                "untrusted",
                "other-project",
                "launch-error",
                "other-session",
                "other-session-error",
            ] {
                let opened = Rc::new(Cell::new(false));
                let callback_opened = opened.clone();
                let mut notifications_before = 0;
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.project.repository.execution_allowed = true;
                        notifications_before = app.extensions.active.notifications.len();
                        let requested_project = if scenario == "other-project" {
                            project.join("other")
                        } else {
                            app.workspace_project()
                        };
                        let path = if scenario == "other-session-error" {
                            project.parent().unwrap().join("outside-repository.rs")
                        } else {
                            project.canonicalize().unwrap().join("file.rs")
                        };
                        prepare_diff(
                            requested_project,
                            app.composer.sessions.current_target().to_owned(),
                            path,
                            "Fixture editor",
                            window,
                            cx,
                            move |_, base, _, _| {
                                assert!(base.path().is_file());
                                assert_eq!(std::fs::read(base.path()).unwrap(), b"");
                                callback_opened.set(true);
                                if scenario == "launch-error" {
                                    Err("fixture launch failure".into())
                                } else {
                                    Ok(())
                                }
                            },
                        );
                        assert!(!opened.get(), "open must wait for background preparation");
                        if scenario == "untrusted" {
                            app.project.repository.execution_allowed = false;
                        }
                        if matches!(scenario, "other-session" | "other-session-error") {
                            let project_before = app.workspace_project();
                            app.composer.sessions.switch_to(
                                format!("draft:{scenario}"),
                                app.composer.sessions.current(),
                            );
                            assert_eq!(app.workspace_project(), project_before);
                        }
                    });
                });
                cx.run_until_parked();
                assert_eq!(opened.get(), matches!(scenario, "current" | "launch-error"));
                if matches!(scenario, "other-session" | "other-session-error") {
                    cx.update(|_, cx| {
                        assert_eq!(
                            app.read(cx).extensions.active.notifications.len(),
                            notifications_before,
                            "stale preparation must not notify the new session"
                        );
                    });
                }
                if matches!(scenario, "untrusted" | "launch-error") {
                    let expected = if scenario == "untrusted" {
                        "Project trust changed"
                    } else {
                        "fixture launch failure"
                    };
                    cx.update(|_, cx| {
                        assert!(
                            app.read(cx)
                                .extensions
                                .active
                                .notifications
                                .iter()
                                .any(|notice| notice.message.contains(expected))
                        );
                    });
                }
            }
        },
    );
}
