use super::*;

#[test]
fn codex_failed_turn_retry_keeps_selected_model_and_effort() {
    isolated_title(
        "selection_lifecycle_tests::codex_failed_turn_retry_keeps_selected_model_and_effort",
        || {
            let mut scenario = Scenario::new(Backend::Codex, Some("Existing chat"), true);
            let model: Model = serde_json::from_value(json!({
                "id":"gpt-6-sol", "name":"Sol", "provider":"openai"
            }))
            .expect("selected model");
            scenario.owner.set_model(model.clone());
            scenario.owner.set_thinking("high".into());
            scenario.until(|s| {
                !s.owner.pending_session_controls.selection_pending()
                    && s.owner
                        .snapshot
                        .session_identity()
                        .model
                        .is_some_and(|m| m.id == model.id)
                    && s.owner.snapshot.session_identity().effort == Some("high")
            });
            scenario
                .backend
                .state
                .lock()
                .expect("fixture state")
                .fail_turn = true;
            scenario.prompt();
            assert_eq!(scenario.owner.snapshot.status, "Failed");
            assert!(scenario.owner.process.is_none());

            scenario
                .backend
                .state
                .lock()
                .expect("fixture state")
                .fail_turn = false;
            scenario.prompt();
            let state = scenario.backend.state.lock().expect("fixture state");
            let turns: Vec<_> = state
                .requests
                .iter()
                .filter(|request| request["method"] == "turn/start")
                .collect();
            assert_eq!(turns.len(), 2);
            for turn in turns {
                assert_eq!(turn["params"]["model"], "gpt-6-sol");
                assert_eq!(turn["params"]["effort"], "high");
            }
            assert_eq!(
                scenario
                    .owner
                    .snapshot
                    .session_identity()
                    .model
                    .map(|m| m.id.as_str()),
                Some("gpt-6-sol")
            );
            assert_eq!(
                scenario.owner.snapshot.session_identity().effort,
                Some("high")
            );
        },
    );
}

#[test]
fn codex_restart_keeps_newer_unacknowledged_selection() {
    isolated_title(
        "selection_lifecycle_tests::codex_restart_keeps_newer_unacknowledged_selection",
        || {
            let mut scenario = Scenario::new(Backend::Codex, Some("Existing chat"), true);
            scenario.owner.set_thinking("low".into());
            scenario.until(|s| s.owner.snapshot.session_identity().effort == Some("low"));
            let model: Model = serde_json::from_value(json!({
                "id":"gpt-6-sol", "name":"Sol", "provider":"openai"
            }))
            .expect("selected model");
            scenario.owner.set_model(model);
            scenario.owner.set_thinking("high".into());
            // Restart before either selection acknowledgement reaches the runtime.
            scenario
                .owner
                .start_process(scenario.owner.active_session.clone());
            scenario.prompt();
            let state = scenario.backend.state.lock().expect("fixture state");
            let turn = state
                .requests
                .iter()
                .find(|r| r["method"] == "turn/start")
                .expect("retry turn");
            assert_eq!(turn["params"]["model"], "gpt-6-sol");
            assert_eq!(turn["params"]["effort"], "high");
            assert_eq!(
                scenario.owner.snapshot.session_identity().effort,
                Some("high")
            );
        },
    );
}
