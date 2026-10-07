use super::*;

#[test]
fn catalog_preserves_router_parameters_effort_and_fast_tier() {
    let models = models(&[
        json!({"id":"auto-smart","displayName":"Router","parameters":[
            {"id":"optimize_for","values":[{"value":"cost"},{"value":"balanced"}]},
            {"id":"reasoning_effort","values":[{"value":"low"},{"value":"high"}]},
            {"id":"fast","values":[{"value":"false"},{"value":"true"}]}
        ]}),
    ]);
    assert_eq!(models.len(), 2);
    let mut model = models[1].clone();
    assert_eq!(model.id, "auto-smart[optimize_for=balanced]");
    assert_eq!(model.parameter("optimize_for").as_deref(), Some("balanced"));
    model.set_parameter("reasoning_effort", "high");
    model.set_parameter("fast", "true");
    assert_eq!(model.effort().as_deref(), Some("high"));
    assert_eq!(model.tier().as_deref(), Some("priority"));
    assert_eq!(
        model.display["serviceTiers"],
        json!(["standard", "priority"])
    );
}

#[test]
fn parameterless_models_do_not_advertise_effort_or_fast() {
    let models = models(&[json!({"id":"plain"})]);
    assert_eq!(models[0].display["serviceTiers"], json!([]));
    assert!(!models[0].display["reasoning"].as_bool().expect("reasoning"));
}

#[test]
fn central_catalog_roundtrip_reconstructs_cursor_requests() {
    let original = models(&[json!({"id":"router","displayName":"Router","parameters":[
        {"id":"optimize_for","values":[{"value":"cost"},{"value":"quality"}]},
        {"id":"reasoning_effort","values":[{"value":"low"},{"value":"high"}]},
        {"id":"fast","values":[{"value":"false"},{"value":"true"}]}
    ]})]);
    let catalog =
        super::super::super::configuration_catalog(metadata(&original, &original[0])).unwrap();
    let persisted = serde_json::to_vec(&catalog).unwrap();
    let catalog = serde_json::from_slice(&persisted).unwrap();
    let restored = from_catalog(&catalog).unwrap();
    assert_eq!(restored.len(), original.len());
    for (mut restored, mut original) in restored.into_iter().zip(original) {
        assert_eq!(restored.id, original.id);
        restored.set_parameter("reasoning_effort", "high");
        restored.set_parameter("fast", "true");
        original.set_parameter("reasoning_effort", "high");
        original.set_parameter("fast", "true");
        assert_eq!(restored.wire, original.wire);
        assert_eq!(restored.effort(), original.effort());
        assert_eq!(restored.tier(), original.tier());
    }
    let mut legacy = catalog;
    legacy.models[0].adapter_data = None;
    assert!(from_catalog(&legacy).is_none());
}
