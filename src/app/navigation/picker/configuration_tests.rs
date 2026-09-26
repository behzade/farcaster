use super::*;
use crate::agents::Backend;

#[test]
fn runtime_selection_matches_model_identity_and_effort() {
    let model = Model {
        id: "id".into(),
        name: "Model".into(),
        provider: "provider".into(),
        context_window: 0,
        reasoning: true,
        resolved_model: None,
        service_tiers: Vec::new(),
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
    assert_eq!(
        selected_row(&rows, &commands, &snapshot, None, None),
        Some(1)
    );
    assert_eq!(selected_row(&[], &commands, &snapshot, None, None), None);
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
        service_tiers: Vec::new(),
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
    .expect("decode fixture model");
    let mut snapshot = crate::runtime::RuntimeSnapshot {
        harness: Some(Backend::OpenCode),
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
    assert_eq!(
        selected_row(&rows, &commands, &snapshot, None, None),
        Some(0)
    );
    snapshot.prefill_thinking_level = Some("high".into());
    assert_eq!(
        selected_row(&rows, &commands, &snapshot, None, None),
        Some(3)
    );
    let rows = effort_picker_rows(&snapshot, &model, &mut commands);
    assert!(
        !rows[0]
            .detail
            .as_deref()
            .expect("row detail")
            .contains("Current")
    );
    assert!(
        rows[3]
            .detail
            .as_deref()
            .expect("row detail")
            .contains("Current")
    );
    snapshot.harness = Some(Backend::Pi);
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
    .expect("decode fixture model");
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
    assert_eq!(
        selected_row(&rows, &commands, &snapshot, None, None),
        Some(1)
    );
}

#[test]
fn harness_selection_matches_the_profile_choice_and_command() {
    let snapshot = crate::runtime::RuntimeSnapshot::default();
    let mut commands = HashMap::new();
    let rows = [
        picker_row(
            &mut commands,
            "harness:pi",
            PickerCommand::SetHarness(Backend::Pi),
            AppIcon::List,
            "Pi",
            Some("Current".into()),
            None,
            "",
        ),
        picker_row(
            &mut commands,
            "profile:first",
            PickerCommand::SetHarnessProfile(Backend::Pi, "first".into()),
            AppIcon::List,
            "First",
            Some("Current".into()),
            None,
            "",
        ),
        picker_row(
            &mut commands,
            "profile:other-backend",
            PickerCommand::SetHarnessProfile(Backend::OpenCode, "target".into()),
            AppIcon::List,
            "Other backend",
            Some("Current".into()),
            None,
            "",
        ),
        picker_row(
            &mut commands,
            "profile:target",
            PickerCommand::SetHarnessProfile(Backend::Pi, "target".into()),
            AppIcon::List,
            "Target",
            Some("Other detail".into()),
            None,
            "",
        ),
    ];

    let selected = selected_row(
        &rows,
        &commands,
        &snapshot,
        Some(Backend::Pi),
        Some("target"),
    );
    assert_eq!(selected, Some(3));
    assert!(matches!(
        commands.get(&rows[selected.expect("profile row")].id),
        Some(PickerCommand::SetHarnessProfile(Backend::Pi, id)) if id == "target"
    ));

    let selected = selected_row(&rows, &commands, &snapshot, Some(Backend::Pi), None);
    assert_eq!(selected, Some(0));
    assert!(matches!(
        commands.get(&rows[selected.expect("stock row")].id),
        Some(PickerCommand::SetHarness(Backend::Pi))
    ));
    assert_eq!(
        selected_row(
            &rows,
            &commands,
            &snapshot,
            Some(Backend::Pi),
            Some("missing")
        ),
        None
    );
}
