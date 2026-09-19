use super::*;

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
    assert!(ids.selections["base"].parameters.is_empty());
    assert_eq!(metadata.service_tier.as_deref(), Some("priority"));
    assert_eq!(metadata.service_tiers, ["standard", "priority"]);
    assert_eq!(ids.service_tier.as_deref(), Some("fast"));
    assert_eq!(metadata.efforts, ["high", "low"]);
}
