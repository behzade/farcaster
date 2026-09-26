use super::*;
use crate::repository::BackendPreference;

#[test]
fn denied_or_unreadable_trust_never_invokes_repository_commands() {
    for allowed in [Ok(false), Err("unreadable trust store".to_owned())] {
        assert!(observe_if_allowed(|| allowed, || panic!("must not execute")).is_none());
    }
    assert!(matches!(
        observe_if_allowed(|| Ok(true), || Some(Ok(None))),
        Some(Ok(None))
    ));
}

#[gpui::test]
fn scheduling_excludes_removed_projects_and_coalesces_busy_ticks(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::scheduling_excludes_removed_projects_and_coalesces_busy_ticks"
        ),
        cx,
        |cx, app, _, project| {
            let second = project.join("second");
            let third = project.join("third");
            let excluded = project.join("excluded");
            app.update(cx, |app, _| {
                app.project.registered = vec![
                    project.to_path_buf(),
                    second.clone(),
                    third.clone(),
                    excluded.clone(),
                ];
                app.project.excluded = vec![excluded.clone()];
                app.project.repository.loading = false;
                assert!(!app.known_repository_projects().contains(&excluded));
                let selected = app.next_offscreen_project().unwrap();
                assert_ne!(selected, project);
                assert_ne!(selected, excluded);
                let ticket = app
                    .project
                    .repository
                    .observations
                    .begin(selected.clone(), BackendPreference::Auto)
                    .unwrap();
                for _ in 0..20 {
                    assert!(app.next_offscreen_project().is_none());
                }
                app.forget_repository_project(&selected);
                assert!(
                    app.next_offscreen_project().is_none(),
                    "invalidated work remains outstanding"
                );
                assert!(!app.project.repository.observations.finish(&ticket));
                assert!(app.next_offscreen_project().is_some());
                app.project.repository.loading = true;
                assert!(
                    app.next_offscreen_project().is_none(),
                    "foreground refresh takes priority"
                );
            });
        },
    );
}

#[gpui::test]
fn untrusted_registered_project_does_not_start_prefetch(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::untrusted_registered_project_does_not_start_prefetch"
        ),
        cx,
        |cx, app, _, project| {
            let target = project.join("untrusted");
            std::fs::create_dir(&target).unwrap();
            app.update(cx, |app, cx| {
                app.project.registered.push(target.clone());
                app.project.repository.loading = false;
                app.prefetch_repository_observation(target.clone(), cx);
                assert!(app.project.repository.warmed.contains(&target));
                assert!(
                    !app.project.repository.observations.busy(),
                    "must not enqueue repository execution"
                );
                assert!(
                    app.project
                        .repository
                        .observations
                        .reuse(&target, BackendPreference::Auto)
                        .is_none()
                );
            });
        },
    );
}
