use super::*;
use crate::repository::BackendPreference;

#[test]
fn denied_or_unreadable_trust_never_invokes_repository_commands() {
    for allowed in [Ok(false), Err("unreadable trust store".to_owned())] {
        assert!(matches!(
            observe_if_allowed(|| allowed.clone(), || panic!("must not execute")),
            PrefetchResult::Untrusted
        ));
    }
    assert!(matches!(
        observe_if_allowed(|| Ok(true), || Some(Ok(None))),
        PrefetchResult::Scanned(Some(Ok(None)))
    ));
}

#[test]
fn prefetch_discards_results_if_trust_changes_during_the_scan() {
    let allowed = std::cell::Cell::new(true);
    let result = observe_if_allowed(
        || Ok(allowed.get()),
        || {
            allowed.set(false);
            Some(Ok(None))
        },
    );
    assert!(matches!(result, PrefetchResult::Untrusted));
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
                assert!(app.repository_project_is_known(&second));
                assert!(!app.repository_project_is_known(&excluded));
                assert!(!app.repository_project_is_known(&project.join("unknown")));
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
fn hover_defers_the_trust_check_and_discards_an_untrusted_observation(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::hover_defers_the_trust_check_and_discards_an_untrusted_observation"
        ),
        cx,
        |cx, app, _, project| {
            let target = project.join("untrusted");
            std::fs::create_dir(&target).unwrap();
            let lock = crate::app::paths::data_dir()
                .unwrap()
                .join("project-trust.json.lock");
            cx.update(|_, cx| {
                app.update(cx, |app, cx| {
                    std::fs::create_dir(&lock).unwrap();
                    app.project.registered.push(target.clone());
                    app.project.repository.observations.remember(
                        target.clone(),
                        super::super::tests::cached_observation(BackendPreference::Auto),
                    );
                    assert!(
                        app.project
                            .repository
                            .observations
                            .snapshot(&target)
                            .is_some()
                    );
                    app.project.repository.loading = false;
                    app.prefetch_repository_observation(target.clone(), cx);
                    assert!(app.project.repository.warmed.contains(&target));
                    assert!(
                        app.project.repository.observations.busy(),
                        "the hover queued the check while the lock was held"
                    );
                    std::fs::remove_dir(&lock).unwrap();
                })
            });
            cx.run_until_parked();
            app.update(cx, |app, _| {
                assert!(!app.project.repository.observations.busy());
                assert!(
                    app.project
                        .repository
                        .observations
                        .snapshot(&target)
                        .is_none()
                );
            });
        },
    );
}
