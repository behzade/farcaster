use super::*;

#[test]
fn saving_one_route_preserves_other_routes_with_incomplete_edits() {
    let task = WorkerTaskDefinition::new("audit".into());
    let mut editor = WorkerTaskEditor {
        tasks: vec![task.clone()],
        saved: vec![task.clone()],
        ..Default::default()
    };
    editor.tasks[0].guided.provider.clear();
    editor.tasks[0].specified.model = "another-model".into();
    let saved = editor
        .route_settings(WorkerRouteTarget {
            task: 0,
            judgment: WorkerJudgment::Specified,
        })
        .unwrap();
    assert_eq!(saved[0].specified.model, "another-model");
    assert_eq!(saved[0].guided, task.guided);
    assert!(
        editor
            .route_settings(WorkerRouteTarget {
                task: 0,
                judgment: WorkerJudgment::Guided
            })
            .is_err()
    );
    assert_eq!(editor.saved, vec![task]);
}

#[test]
fn worker_route_changes_clear_only_downstream_choices() {
    let mut route = WorkerTaskDefinition::new("read".into()).specified;
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
    let mut editor = WorkerTaskEditor::default();
    assert!(editor.save_name(None, "bad name").is_err());
    assert!(editor.tasks.is_empty());
    editor.save_name(None, "audit").unwrap();
    assert!(editor.save_name(None, "AUDIT").is_err());
    editor.save_name(Some(0), "review").unwrap();
    assert_eq!(editor.tasks.len(), 1);
    assert_eq!(editor.tasks[0].name, "review");
    let target = WorkerRouteTarget {
        task: 0,
        judgment: WorkerJudgment::Guided,
    };
    let original = editor.tasks[0].guided.clone();
    assert!(
        editor
            .save_custom_route(target, ["provider".into(), String::new(), "high".into()])
            .is_err()
    );
    assert_eq!(editor.tasks[0].guided, original);
    editor
        .save_custom_route(
            target,
            ["provider".into(), "custom-model".into(), String::new()],
        )
        .unwrap();
    assert_eq!(editor.tasks[0].guided.harness, original.harness);
    assert_eq!(editor.tasks[0].guided.model, "custom-model");
    assert_eq!(editor.tasks[0].guided.effort, None);
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
    let editor = WorkerTaskEditor {
        catalogs: vec![
            entry("pi", "/project", &["low", "medium", "high"]),
            entry("pi", "/other", &["wrong"]),
            entry("codex-cli", "/project", &["wrong"]),
            entry("pi", "/project", &["high"]),
        ],
        ..WorkerTaskEditor::default()
    };
    assert_eq!(
        editor.catalog("pi", Path::new("/project")).efforts,
        ["low", "medium", "high"]
    );
}

#[test]
fn worker_efforts_follow_the_selected_model_not_the_harness_alone() {
    let route = WorkerTaskDefinition::new("read".into()).specified;
    let mut catalog = ConfigurationCatalog {
        models: vec![crate::protocol::Model {
            id: route.model.clone(),
            name: "Luna".into(),
            provider: route.provider.clone(),
            context_window: 0,
            reasoning: true,
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
