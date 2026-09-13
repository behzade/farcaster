use super::*;

#[test]
fn runtime_selection_matches_model_identity_and_effort() {
    let model = Model {
        id: "id".into(),
        name: "Model".into(),
        provider: "provider".into(),
        context_window: 0,
        reasoning: true,
        resolved_model: None,
        access_modes: None,
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
    assert_eq!(selected_row(&rows, &commands, &snapshot, None), Some(1));
    assert_eq!(selected_row(&[], &commands, &snapshot, None), None);
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
        resolved_model: None,
        access_modes: None,
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

#[test]
fn opencode_default_row_is_first_and_matches_only_the_unset_variant() {
    let model: Model = serde_json::from_value(serde_json::json!({
        "id": "test", "name": "Test", "provider": "provider", "reasoning": true,
        "efforts": ["thinking", "high", "none", "low"]
    }))
    .unwrap();
    let mut snapshot = crate::runtime::RuntimeSnapshot {
        harness: "opencode2".into(),
        prefill_model: Some(model.clone()),
        ..Default::default()
    };
    let mut commands = HashMap::new();
    let rows = effort_picker_rows(&snapshot, &model, &mut commands);
    assert_eq!(
        rows.iter()
            .map(|row| row.label.as_str())
            .collect::<Vec<_>>(),
        ["Default", "none", "low", "high", "thinking"]
    );
    assert!(matches!(
        commands.get(&rows[0].id),
        Some(PickerCommand::SetRuntime { effort: None, .. })
    ));
    assert_eq!(selected_row(&rows, &commands, &snapshot, None), Some(0));
    snapshot.prefill_thinking_level = Some("high".into());
    assert_eq!(selected_row(&rows, &commands, &snapshot, None), Some(3));
    let rows = effort_picker_rows(&snapshot, &model, &mut commands);
    assert!(!rows[0].detail.as_deref().unwrap().contains("Current"));
    assert!(rows[3].detail.as_deref().unwrap().contains("Current"));
    snapshot.harness = "pi".into();
    assert_eq!(
        effort_picker_rows(&snapshot, &model, &mut commands).len(),
        4
    );
}

#[test]
fn direct_model_rows_still_select_a_non_reasoning_model_with_a_reported_level() {
    let model: Model = serde_json::from_value(serde_json::json!({
        "id": "plain", "name": "Plain", "provider": "provider", "reasoning": false
    }))
    .unwrap();
    let snapshot = crate::runtime::RuntimeSnapshot {
        prefill_model: Some(model.clone()),
        prefill_thinking_level: Some("off".into()),
        ..Default::default()
    };
    let mut commands = HashMap::new();
    let rows = ["other", "plain"].map(|id| {
        picker_row(
            &mut commands,
            id,
            PickerCommand::SetRuntime {
                model: Model {
                    id: id.into(),
                    ..model.clone()
                },
                effort: None,
            },
            AppIcon::List,
            id,
            None,
            None,
            "",
        )
    });
    assert_eq!(selected_row(&rows, &commands, &snapshot, None), Some(1));
}
