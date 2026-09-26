use super::*;

fn test_route(model: &str) -> WorkerExecution {
    WorkerExecution {
        harness: Backend::Pi,
        provider: "openai".into(),
        model: model.into(),
        effort: None,
        service_tier: None,
    }
}

#[gpui::test]
fn saving_first_model_in_empty_profile_persists_it(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::saving_first_model_in_empty_profile_persists_it"
        ),
        cx,
        |_, _, _, _| {
            let settings = crate::app::persistence::open()
                .unwrap()
                .load_worker_profiles()
                .unwrap();
            let mut draft = settings.profiles.clone();
            draft[0].models.push(test_route("first"));
            let mut editor = WorkerProfileEditor {
                profiles: draft,
                saved: settings.profiles,
                inherit_limit: settings.inherit_limit,
                inherit_enabled: settings.inherit_enabled,
                loaded: true,
                ..WorkerProfileEditor::default()
            };
            editor
                .persist_route(WorkerRouteTarget {
                    profile: 0,
                    model: 0,
                })
                .unwrap();
            let stored = crate::app::persistence::open()
                .unwrap()
                .load_worker_profiles()
                .unwrap();
            assert_eq!(stored.profiles[0].models, vec![test_route("first")]);
            assert_eq!(editor.saved[0].models, stored.profiles[0].models);
        },
    );
}

#[gpui::test]
fn saving_existing_model_replaces_it(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(module_path!(), "::saving_existing_model_replaces_it"),
        cx,
        |_, _, _, _| {
            let mut settings = crate::app::persistence::open()
                .unwrap()
                .load_worker_profiles()
                .unwrap();
            settings.profiles[0].models.push(test_route("old"));
            crate::app::persistence::open()
                .unwrap()
                .save_worker_profiles(&settings)
                .unwrap();
            let mut draft = settings.profiles.clone();
            draft[0].models[0] = test_route("new");
            let mut editor = WorkerProfileEditor {
                profiles: draft,
                saved: settings.profiles,
                inherit_limit: settings.inherit_limit,
                inherit_enabled: settings.inherit_enabled,
                loaded: true,
                ..WorkerProfileEditor::default()
            };
            editor
                .persist_route(WorkerRouteTarget {
                    profile: 0,
                    model: 0,
                })
                .unwrap();
            let stored = crate::app::persistence::open()
                .unwrap()
                .load_worker_profiles()
                .unwrap();
            assert_eq!(stored.profiles[0].models, vec![test_route("new")]);
        },
    );
}

#[gpui::test]
fn saving_invalid_model_index_does_not_expand_profile(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::saving_invalid_model_index_does_not_expand_profile"
        ),
        cx,
        |_, _, _, _| {
            let settings = crate::app::persistence::open()
                .unwrap()
                .load_worker_profiles()
                .unwrap();
            let mut draft = settings.profiles.clone();
            draft[0].models = vec![test_route("first"), test_route("second")];
            let mut editor = WorkerProfileEditor {
                profiles: draft,
                saved: settings.profiles.clone(),
                inherit_limit: settings.inherit_limit,
                inherit_enabled: settings.inherit_enabled,
                loaded: true,
                ..WorkerProfileEditor::default()
            };
            assert_eq!(
                editor.persist_route(WorkerRouteTarget {
                    profile: 0,
                    model: 1
                }),
                Err("Model no longer exists".into())
            );
            assert_eq!(editor.saved, settings.profiles);
            let stored = crate::app::persistence::open()
                .unwrap()
                .load_worker_profiles()
                .unwrap();
            assert_eq!(stored.profiles, settings.profiles);
        },
    );
}

#[test]
fn profile_editor_keeps_one_route() {
    let mut models = Vec::new();
    assert_eq!(edit_models(&mut models, 0, WorkerModelEdit::Add), Ok(0));
    assert_eq!(models.len(), 1);
    assert!(edit_models(&mut models, 0, WorkerModelEdit::Add).is_err());
    assert_eq!(edit_models(&mut models, 0, WorkerModelEdit::Remove), Ok(0));
    assert!(models.is_empty());
}

#[test]
fn changing_a_route_clears_incompatible_choices() {
    let mut route = WorkerExecution {
        harness: Backend::Cursor,
        provider: "cursor-cli".into(),
        model: "first".into(),
        effort: Some("high".into()),
        service_tier: Some("fast".into()),
    };
    apply_choice(
        &mut route,
        WorkerRouteChoice::Model {
            provider: "cursor-cli".into(),
            id: "second".into(),
        },
    );
    assert_eq!(route.model, "second");
    assert_eq!(route.effort, None);
    assert_eq!(route.service_tier, None);
}
