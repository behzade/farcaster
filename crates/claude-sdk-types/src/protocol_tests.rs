use super::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

fn round_trip<T: DeserializeOwned + Serialize>(value: &Value) {
    let parsed: T = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), *value);
}

fn user_message() -> Value {
    json!({
        "type": "user", "message": {"role": "user", "content": "hello"},
        "parent_tool_use_id": null
    })
}

#[test]
fn cli_compatibility_only_relaxes_absent_metadata() {
    let mut text = json!({"type":"text","text":"hello"});
    let usage = json!({"input_tokens":2,"output_tokens":1});
    let thinking = json!({"type":"thinking","thinking":""});
    #[cfg(feature = "cli-compat")]
    {
        round_trip::<BetaTextBlock>(&text);
        round_trip::<NonNullableUsage>(&usage);
        round_trip::<BetaThinkingBlock>(&thinking);
    }
    #[cfg(not(feature = "cli-compat"))]
    {
        assert!(serde_json::from_value::<BetaTextBlock>(text.clone()).is_err());
        assert!(serde_json::from_value::<NonNullableUsage>(usage).is_err());
        assert!(serde_json::from_value::<BetaThinkingBlock>(thinking).is_err());
    }
    text["citations"] = json!(905);
    assert!(
        serde_json::from_value::<BetaThinkingBlock>(
            json!({"type":"thinking","thinking":"","signature":905})
        )
        .is_err()
    );
    assert!(serde_json::from_value::<BetaTextBlock>(text.clone()).is_err());
    text["citations"] = Value::Null;
    text.as_object_mut().unwrap().remove("text");
    assert!(serde_json::from_value::<BetaTextBlock>(text).is_err());
    assert!(
        serde_json::from_value::<NonNullableUsage>(json!({"input_tokens":"bad","output_tokens":1}))
            .is_err()
    );
}

#[test]
fn source_checked_fixtures_round_trip_without_losing_fields() {
    let fixtures: Value = serde_json::from_str(include_str!("../fixtures/protocol.json")).unwrap();
    for fixture in fixtures.as_array().unwrap() {
        let value = &fixture["value"];
        match fixture["rust_type"].as_str().unwrap() {
            "SDKControlRequest" => round_trip::<SDKControlRequest>(value),
            "SDKControlResponse" => round_trip::<SDKControlResponse>(value),
            "SDKUserMessage" => round_trip::<SDKUserMessage>(value),
            "SDKUserMessageReplay" => round_trip::<SDKUserMessageReplay>(value),
            "SDKPartialAssistantMessage" => round_trip::<SDKPartialAssistantMessage>(value),
            "SDKAssistantMessage" => round_trip::<SDKAssistantMessage>(value),
            "SDKResultSuccess" => round_trip::<SDKResultSuccess>(value),
            other => panic!("unhandled fixture type: {other}"),
        }
        round_trip::<StdoutMessage>(value);
    }
}

#[test]
fn user_replay_selects_the_more_specific_union_member() {
    let mut value = user_message();
    value["uuid"] = json!("00000000-0000-4000-8000-000000000001");
    value["session_id"] = json!("session-1");
    value["isReplay"] = json!(true);
    assert!(matches!(
        serde_json::from_value::<SDKMessage>(value).unwrap(),
        SDKMessage::SDKUserMessageReplay(_)
    ));
}

#[test]
fn missing_required_nullable_property_is_not_json_null() {
    let mut value = user_message();
    round_trip::<SDKUserMessage>(&value);
    value.as_object_mut().unwrap().remove("parent_tool_use_id");
    assert!(serde_json::from_value::<SDKUserMessage>(value).is_err());
}

#[test]
fn optional_unknown_preserves_missing_null_and_structured_values() {
    let base = user_message();
    round_trip::<SDKUserMessage>(&base);
    for metadata in [Value::Null, json!({"file_path": 905}), json!([1, false])] {
        let mut value = base.clone();
        value["tool_use_result"] = metadata;
        round_trip::<SDKUserMessage>(&value);
    }
}

#[test]
fn future_object_fields_survive_but_wrong_known_field_types_fail() {
    let mut value = json!({"subtype": "seed_read_state", "path": "/project/file", "mtime": 1, "new_cli_metadata": {"count": 7}});
    round_trip::<SDKControlSeedReadStateRequest>(&value);
    for invalid in [json!(905), json!(false), Value::Null, json!([]), json!({})] {
        value["path"] = invalid;
        assert!(serde_json::from_value::<SDKControlSeedReadStateRequest>(value.clone()).is_err());
    }
}

#[test]
fn wrong_literal_discriminants_and_missing_control_fields_fail() {
    let valid =
        json!({"type": "control_request", "request_id": "1", "request": {"subtype": "interrupt"}});
    round_trip::<SDKControlRequest>(&valid);
    for invalid in [
        json!({"type": "control_response", "request_id": "1", "request": {"subtype": "interrupt"}}),
        json!({"type": "control_request", "request": {"subtype": "interrupt"}}),
        json!({"type": "control_request", "request_id": "1", "request": {"subtype": "made_up"}}),
    ] {
        assert!(serde_json::from_value::<SDKControlRequest>(invalid).is_err());
    }
}
