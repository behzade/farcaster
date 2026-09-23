use super::*;

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
