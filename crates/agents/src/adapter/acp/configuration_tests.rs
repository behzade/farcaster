use super::*;

#[test]
fn context_variants_have_readable_names_and_keep_exact_selection_parameters() {
    assert_eq!(
        display_model_name("Base · 300k context"),
        "Base · 300k context"
    );
    assert_eq!(
        display_model_name("Base · context=1m,variant=beta"),
        "Base · 1m context, variant=beta"
    );
    let profile = &super::super::super::cursor::PROFILE;
    let response = json!({"configOptions":[
        {"id":"model","category":"model","currentValue":"base"},
        {"id":"context","category":"model_config","currentValue":"300k"}
    ]});
    let catalog = vec![json!({"value":"base","name":"Base","configOptions":[
        {"id":"context","category":"model_config","options":[{"value":"300k"},{"value":"1m"}]}
    ]})];
    let (metadata, ids) = metadata(profile, &response, catalog);
    assert_eq!(metadata.models[0]["id"], "base[context=300k]");
    assert_eq!(metadata.models[0]["name"], "Base · 300k context");
    assert_eq!(metadata.models[1]["name"], "Base · 1m context");
    assert_eq!(metadata.models[0]["contextWindow"], 300_000);
    assert_eq!(metadata.models[1]["contextWindow"], 1_000_000);
    assert_eq!(ids.selections["base[context=300k]"].model, "base");
    assert_eq!(
        ids.selections["base[context=300k]"].parameters,
        [("context".into(), "300k".into())]
    );
}

#[test]
fn model_parameters_become_neutral_choices_and_model_specific_efforts() {
    let profile = &super::super::super::cursor::PROFILE;
    let response = json!({"configOptions":[
        {"id":"model","category":"model","currentValue":"base","options":[{"value":"base","name":"Base"}]},
        {"id":"fast","category":"model_config","currentValue":"true","options":[{"value":"false"},{"value":"true"}]},
        {"id":"effort","category":"thought_level","currentValue":"high","options":[{"value":"low"},{"value":"high"}]}
    ]});
    let catalog = vec![json!({"value":"base","name":"Base","configOptions":[
        {"id":"fast","category":"model_config","options":[{"value":"false"},{"value":"true"}]},
        {"id":"effort","category":"thought_level","options":[{"value":"low"},{"value":"high"}]}
    ]})];
    let (metadata, ids) = metadata(profile, &response, catalog);
    assert_eq!(metadata.models.len(), 1);
    assert_eq!(metadata.models[0]["id"], "base");
    assert_eq!(metadata.models[0]["efforts"], json!(["low", "high"]));
    assert_eq!(
        metadata.models[0]["serviceTiers"],
        json!(["standard", "priority"])
    );
    assert!(ids.selections["base"].parameters.is_empty());
    assert_eq!(metadata.service_tier.as_deref(), Some("priority"));
    assert_eq!(metadata.service_tiers, ["standard", "priority"]);
    assert_eq!(ids.service_tier.as_deref(), Some("fast"));
    assert_eq!(metadata.efforts, ["high", "low"]);
}

#[test]
fn cursor_tiers_are_advertised_only_for_models_with_fast_option() {
    let profile = &super::super::super::cursor::PROFILE;
    let response = json!({"configOptions":[]});
    let catalog = vec![
        json!({"value":"fast-model","configOptions":[
            {"id":"fast","category":"model_config","options":[{"value":"false"},{"value":"true"}]}
        ]}),
        json!({"value":"plain-model","configOptions":[]}),
    ];
    let (metadata, _) = metadata(profile, &response, catalog);
    assert_eq!(
        metadata.models[0]["serviceTiers"],
        json!(["standard", "priority"])
    );
    assert_eq!(metadata.models[1]["serviceTiers"], json!([]));
}
