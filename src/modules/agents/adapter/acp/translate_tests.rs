use super::*;

#[test]
fn grouped_and_legacy_models_preserve_the_selected_model() {
    let (metadata, ids) = metadata_from_session(
        &PROFILE,
        &json!({"configOptions":[
            {"id":"model","category":"model","currentValue":"second","options":[
                {"group":"models","name":"Models","options":[{"value":"first","name":"First"},{"value":"second","name":"Second"}]}
            ]}
        ]}),
    );
    assert_eq!(metadata.models.len(), 2);
    assert_eq!(metadata.models[0]["id"], "second");
    assert_eq!(ids.model.as_deref(), Some("model"));
    let (metadata, ids) = metadata_from_session(
        &PROFILE,
        &json!({"models":{
            "currentModelId":"second","availableModels":[{"modelId":"first","name":"First"},{"modelId":"second","name":"Second"}]
        }}),
    );
    assert_eq!(metadata.models[0]["id"], "second");
    assert_eq!(ids.selected_model.as_deref(), Some("second"));
    assert!(ids.model.is_none());
}

const PROFILE: AcpProfile = AcpProfile {
    backend: "test-acp",
    name: "Test ACP",
    command: "test-acp",
    path_environment: "FARCASTER_TEST_ACP_PATH",
    arguments: &["acp"],
    auth_method: None,
    force_argument: Some("--force"),
    resume_method: "session/load",
    permission_modes: None,
};

#[test]
fn session_config_options_become_neutral_catalogs() {
    let (metadata, ids) = metadata_from_session(
        &PROFILE,
        &json!({
            "configOptions": [
                {"id":"mode","category":"mode","options":[{"value":"agent","name":"Agent"}]},
                {"id":"model","category":"model","options":[{"value":"fast","name":"Fast"}]}
            ]
        }),
    );
    assert_eq!(metadata.models[0]["id"], "fast");
    assert_eq!(metadata.models[0]["provider"], "test-acp");
    assert_eq!(metadata.modes[0]["id"], "agent");
    assert_eq!(ids.model.as_deref(), Some("model"));
}

#[test]
fn tool_content_unwraps_acp_content_blocks() {
    assert_eq!(
        tool_content(&json!({
            "content": [{
                "type": "content",
                "content": {"type": "text", "text": "done"}
            }]
        })),
        json!([{"type": "text", "text": "done"}])
    );
}

#[test]
fn tool_metadata_merges_partial_acp_updates() {
    let mut metadata = tool_metadata(&json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "one",
        "kind": "read",
        "title": "Read file",
        "locations": [{"path": "src/main.rs", "line": 4}]
    }));
    merge_tool_metadata(
        &mut metadata,
        &json!({"sessionUpdate":"tool_call_update", "rawInput":{"path":"src/main.rs"}}),
    );
    assert_eq!(metadata.category, Some(ToolCategory::Read));
    assert_eq!(metadata.title.as_deref(), Some("Read file"));
    assert_eq!(metadata.targets, ["src/main.rs"]);
    assert_eq!(tool_args(&metadata), json!({"path":"src/main.rs"}));
    assert_eq!(
        metadata
            .native
            .as_ref()
            .expect("test operation should succeed")["kind"],
        "read"
    );
    assert_eq!(
        metadata
            .native
            .as_ref()
            .expect("test operation should succeed")["rawInput"]["path"],
        "src/main.rs"
    );
}

#[test]
fn completed_update_retains_content_from_an_earlier_partial_update() {
    let mut metadata = tool_metadata(&json!({
        "sessionUpdate":"tool_call_update",
        "toolCallId":"one",
        "content":[{"type":"text", "text":"earlier output"}]
    }));
    let completed = json!({
        "sessionUpdate":"tool_call_update",
        "toolCallId":"one",
        "status":"completed"
    });
    merge_tool_metadata(&mut metadata, &completed);
    assert_eq!(
        merged_tool_content(&metadata, &completed),
        json!([{"type":"text", "text":"earlier output"}])
    );
}

#[test]
fn edit_payloads_use_canonical_args_and_diff_details() {
    assert_eq!(
        tool_args(&tool_metadata(&json!({
            "kind": "edit",
            "rawInput": {
                "filePath": "src/main.rs",
                "old_string": "old",
                "new_string": "new\nline"
            }
        }))),
        json!({
            "path": "src/main.rs",
            "oldText": "old",
            "newText": "new\nline"
        })
    );

    let metadata = tool_metadata(&json!({
        "kind": "edit",
        "locations": [{"path": "src/lib.rs", "line": 10}],
        "rawInput": {"path": "src/lib.rs"},
        "content": [{
            "type": "diff",
            "path": "src/lib.rs",
            "oldText": "fn a() {}\nfn keep() {}",
            "newText": "fn a() {}\nfn keep() {}\nfn b() {}"
        }]
    }));
    assert_eq!(tool_args(&metadata), json!({"path": "src/lib.rs"}));
    assert_eq!(
        tool_result(&metadata, &json!({}))["details"],
        json!({
            "diff": "+fn b() {}\n",
            "firstChangedLine": 10
        })
    );
}

#[test]
fn full_file_acp_diffs_count_only_changed_lines() {
    let old = (0..80)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut new_lines: Vec<String> = (0..80).map(|n| format!("line {n}")).collect();
    new_lines[10] = "changed".into();
    new_lines.insert(40, "inserted".into());
    let metadata = tool_metadata(&json!({
        "kind": "edit",
        "content": [{
            "type": "diff",
            "path": "big.rs",
            "oldText": old,
            "newText": new_lines.join("\n")
        }]
    }));
    assert_eq!(
        tool_result(&metadata, &json!({}))["details"]["diff"],
        json!("-line 10\n+changed\n+inserted\n")
    );
}

#[test]
fn execute_kind_does_not_guess_bash() {
    assert_eq!(
        normalize_tool_name(&json!({"kind":"execute"}), "Run database migration"),
        "Run database migration"
    );
}

#[test]
fn available_commands_update_becomes_prompt_commands() {
    let message = super::super::events::AcpInbound::Notification {
        method: "session/update".into(),
        params: json!({
            "sessionId": "one",
            "update": {
                "sessionUpdate": "available_commands_update",
                "availableCommands": [{"name":"/review","description":"Review changes"}]
            }
        }),
    };
    assert_eq!(
        commands_from_update(&message, "one"),
        Some(vec![json!({
            "name": "review",
            "description": "Review changes",
            "source": "prompt"
        })])
    );
    assert_eq!(commands_from_update(&message, "other"), None);
}

#[test]
fn cursor_multi_select_accepts_include_but_not_skip() {
    assert!(is_acceptance("Include"));
    assert!(!is_acceptance("Skip"));
}
