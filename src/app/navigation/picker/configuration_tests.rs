use super::*;

#[test]
fn runtime_selection_matches_model_identity_and_effort() {
    let model = Model {
        id: "id".into(),
        name: "Model".into(),
        provider: "provider".into(),
        context_window: 0,
        reasoning: true,
        efforts: None,
    };
    let snapshot = crate::runtime::RuntimeSnapshot {
        prefill_model: Some(model.clone()),
        prefill_thinking_level: Some("high".into()),
        ..Default::default()
    };
    let mut commands = HashMap::new();
    let rows = ["low", "high"].map(|effort| {
        picker_row(
            &mut commands,
            effort,
            PickerCommand::SetRuntime {
                model: model.clone(),
                effort: Some(effort.into()),
            },
            AppIcon::List,
            effort,
            None,
            None,
            "",
        )
    });
    assert_eq!(selected_row(&rows, &commands, &snapshot), Some(1));
    assert_eq!(selected_row(&[], &commands, &snapshot), None);
}

#[test]
fn effort_choices_respect_each_models_limits() {
    let catalog = vec!["low".into(), "high".into()];
    let mut model = Model {
        id: "test".into(),
        name: "Test".into(),
        provider: "test".into(),
        context_window: 0,
        reasoning: false,
        efforts: None,
    };
    assert!(model_efforts(&model, &catalog).is_empty());
    model.reasoning = true;
    assert_eq!(model_efforts(&model, &catalog), catalog);
    model.efforts = Some(vec!["high".into()]);
    assert_eq!(model_efforts(&model, &catalog), &["high"]);
    model.efforts = Some(vec![]);
    assert!(model_efforts(&model, &catalog).is_empty());
}
