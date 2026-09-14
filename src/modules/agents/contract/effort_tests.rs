use super::*;

#[test]
fn model_choices_are_sorted_without_mutating_or_leaking_catalog_variants() {
    let mut model: Model = serde_json::from_value(serde_json::json!({
        "provider": "openai", "id": "astra", "name": "Astra", "reasoning": true,
        "efforts": ["thinking", "low", "high", "max", "medium", "xhigh", "minimal", "none", "custom"]
    }))
    .expect("decode fixture model");
    let fallback = ["unrelated".into(), "max".into()];
    assert_eq!(
        model_efforts(&model, &fallback),
        [
            "none", "minimal", "low", "medium", "high", "xhigh", "max", "custom", "thinking"
        ]
    );
    assert_eq!(
        model.efforts.as_ref().expect("model efforts")[0],
        "thinking"
    );
    model.efforts = Some(vec![]);
    assert!(model_efforts(&model, &fallback).is_empty());
    model.efforts = None;
    assert_eq!(model_efforts(&model, &fallback), ["max", "unrelated"]);
    model.reasoning = false;
    assert!(model_efforts(&model, &fallback).is_empty());
}
