use super::*;

#[test]
fn model_list_edits_preserve_order_and_keep_at_least_one_choice() {
    let mut models = WorkerProfile::new("fast".into()).models;
    let original = models.clone();
    assert_eq!(
        edit_models(&mut models, 1, WorkerModelEdit::MoveUp).unwrap(),
        0
    );
    assert_eq!(models[0], original[1]);
    assert_eq!(
        edit_models(&mut models, 0, WorkerModelEdit::MoveDown).unwrap(),
        1
    );
    assert_eq!(models, original);
    let added = edit_models(&mut models, 0, WorkerModelEdit::Add).unwrap();
    assert_eq!(models[added], original[0]);
    edit_models(&mut models, added, WorkerModelEdit::Remove).unwrap();
    assert_eq!(models, original);
    models.truncate(1);
    assert!(edit_models(&mut models, 0, WorkerModelEdit::Remove).is_err());
    assert!(edit_models(&mut models, 0, WorkerModelEdit::MoveUp).is_err());
    assert_eq!(models, original[..1]);
}

#[test]
fn saving_one_route_preserves_other_routes_with_incomplete_edits() {
    let profile = WorkerProfile::new("audit".into());
    let mut editor = WorkerProfileEditor {
        profiles: vec![profile.clone(), WorkerProfile::new("other".into())],
        saved: vec![profile.clone(), WorkerProfile::new("other".into())],
        ..Default::default()
    };
    editor.profiles[1].models[0].provider.clear();
    editor.profiles[0].models[1].provider.clear();
    editor.profiles[0].models[0].model = "another-model".into();
    let saved = editor
        .route_settings(WorkerRouteTarget {
            profile: 0,
            model: 0,
        })
        .unwrap();
    assert_eq!(saved[0].models[0].model, "another-model");
    assert_eq!(saved[1], editor.saved[1]);
    assert_eq!(saved[0].models[1], editor.saved[0].models[1]);
    assert!(
        editor
            .route_settings(WorkerRouteTarget {
                profile: 1,
                model: 0
            })
            .is_err()
    );
    assert_eq!(editor.saved[0], profile);
}

#[test]
fn worker_route_changes_clear_only_downstream_choices() {
    let mut route = WorkerProfile::new("read".into()).models[0].clone();
    let original = route.clone();
    apply_choice(
        &mut route,
        WorkerRouteChoice::Provider(original.provider.clone()),
    );
    assert_eq!(route, original);
    apply_choice(
        &mut route,
        WorkerRouteChoice::Model {
            provider: original.provider.clone(),
            id: original.model.clone(),
        },
    );
    assert_eq!(route, original);
    apply_choice(
        &mut route,
        WorkerRouteChoice::Model {
            provider: original.provider.clone(),
            id: "another-model".into(),
        },
    );
    assert_eq!(route.provider, original.provider);
    assert_eq!(route.effort, None);
    apply_choice(&mut route, WorkerRouteChoice::Provider("other".into()));
    assert_eq!(route.harness, original.harness);
    assert!(route.model.is_empty());
    assert_eq!(route.effort, None);
    apply_choice(&mut route, WorkerRouteChoice::Harness("codex-cli".into()));
    assert!(route.provider.is_empty());
}

#[test]
fn worker_task_edits_validate_before_mutating() {
    let mut editor = WorkerProfileEditor::default();
    assert!(editor.save_name(None, "bad name").is_err());
    assert!(editor.profiles.is_empty());
    editor.save_name(None, "audit").unwrap();
    assert!(editor.save_name(None, "AUDIT").is_err());
    editor.save_name(Some(0), "review").unwrap();
    assert_eq!(editor.profiles.len(), 1);
    assert_eq!(editor.profiles[0].name, "review");
    let target = WorkerRouteTarget {
        profile: 0,
        model: 0,
    };
    let original = editor.profiles[0].models[0].clone();
    assert!(
        editor
            .save_custom_route(target, ["provider".into(), String::new(), "high".into()])
            .is_err()
    );
    assert_eq!(editor.profiles[0].models[0], original);
    editor
        .save_custom_route(
            target,
            ["provider".into(), "custom-model".into(), String::new()],
        )
        .unwrap();
    assert_eq!(editor.profiles[0].models[0].harness, original.harness);
    assert_eq!(editor.profiles[0].models[0].model, "custom-model");
    assert_eq!(editor.profiles[0].models[0].effort, None);
}

#[test]
fn worker_catalogs_preserve_effort_order_and_project_scope() {
    let entry = |harness: &str, project: &str, efforts: &[&str]| {
        crate::app::persistence::CachedConfigurationCatalog {
            harness: harness.into(),
            project: project.into(),
            catalog: ConfigurationCatalog {
                models: vec![],
                efforts: efforts.iter().map(|value| (*value).into()).collect(),
            },
        }
    };
    let editor = WorkerProfileEditor {
        catalogs: vec![
            entry("pi", "/project", &["low", "medium", "high"]),
            entry("pi", "/other", &["wrong"]),
            entry("codex-cli", "/project", &["wrong"]),
            entry("pi", "/project", &["high"]),
        ],
        ..WorkerProfileEditor::default()
    };
    assert_eq!(
        editor.catalog("pi", Path::new("/project")).efforts,
        ["low", "medium", "high"]
    );
}

#[test]
fn worker_efforts_follow_the_selected_model_not_the_harness_alone() {
    let route = WorkerProfile::new("read".into()).models[0].clone();
    let mut catalog = ConfigurationCatalog {
        models: vec![crate::protocol::Model {
            id: route.model.clone(),
            name: "Luna".into(),
            provider: route.provider.clone(),
            context_window: 0,
            reasoning: true,
            access_modes: None,
            efforts: Some(vec!["high".into()]),
        }],
        efforts: vec!["low".into(), "high".into()],
    };
    assert_eq!(model_efforts(&catalog, catalog.models.first()), ["high"]);
    catalog.models[0].efforts = Some(vec![]);
    assert!(model_efforts(&catalog, catalog.models.first()).is_empty());
    catalog.models[0].efforts = None;
    assert_eq!(
        model_efforts(&catalog, catalog.models.first()),
        ["low", "high"]
    );
    catalog.models[0].reasoning = false;
    assert!(model_efforts(&catalog, catalog.models.first()).is_empty());
}
