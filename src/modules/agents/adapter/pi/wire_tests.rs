use super::*;
use crate::agents::{SessionHistory, SessionResponsePayload as Payload};

#[test]
fn parses_response_and_activity_frames() {
    assert_eq!(
        parse_frame(br#"{"type":"response","id":"1","command":"abort","success":true}"#),
        Ok(PiWireMessage::Response {
            commands: Vec::new(),
            command: "abort".into(),
            response: crate::agents::SessionResponse::success(
                Some("1".into()),
                crate::agents::SessionResponsePayload::Abort
            ),
        })
    );
    assert_eq!(
        parse_frame(br#"{"type":"agent_start"}"#),
        Ok(PiWireMessage::Event(
            serde_json::json!({"type":"agent_start"})
        ))
    );
}

#[test]
fn normalizes_missing_usage_totals() {
    let PiWireMessage::Event(event) = parse_frame(
        br#"{"type":"turn_end","usage":{"input":10,"output":2,"cacheRead":3,"cacheWrite":1}}"#,
    )
    .expect("turn event") else {
        panic!("expected event");
    };
    assert_eq!(event["usage"]["totalTokens"], 16);

    let PiWireMessage::Response { response, .. } = parse_frame(
            br#"{"type":"response","command":"get_session_stats","success":true,"data":{"tokens":{"input":5,"output":1,"cacheRead":2,"cacheWrite":0}}}"#,
        )
        .expect("usage response")
        else {
            panic!("expected response");
        };
    let Ok(Payload::LoadUsage(usage)) = response.result else {
        panic!("expected usage")
    };
    assert_eq!(usage.tokens.total_tokens, 8);
}

#[test]
fn history_response_projects_pi_entries_before_leaving_adapter() {
    let PiWireMessage::Response { response, .. } = parse_frame(
            br#"{"type":"response","command":"get_entries","success":true,"data":{"entries":[{"type":"message","id":"a","parentId":null,"message":{"role":"user","content":"hello"}}]}}"#,
        ).expect("history") else { panic!("expected response"); };
    let Ok(Payload::LoadHistory(SessionHistory::Replace(messages))) = response.result else {
        panic!("expected history")
    };
    assert_eq!(messages[0]["content"], "hello");
    let PiWireMessage::Response { response, .. } =
        parse_frame(br#"{"type":"response","command":"get_entries","success":true,"data":{}}"#)
            .expect("response envelope")
    else {
        panic!("expected response")
    };
    assert!(response.result.is_err());
}

#[test]
fn adds_per_model_efforts_to_pi_model_catalogs() {
    let frame = serde_json::to_vec(&serde_json::json!({
        "type": "response",
        "command": "get_available_models",
        "success": true,
        "data": {"models": [
            {"id": "plain", "name": "Plain", "provider": "test", "reasoning": false},
            {"id": "default", "name": "Default", "provider": "test", "reasoning": true},
            {"id": "mapped", "name": "Mapped", "provider": "test", "reasoning": true, "thinkingLevelMap": {
                "minimal": null,
                "xhigh": "xhigh",
                "max": null
            }},
            {"id": "future", "name": "Future", "provider": "test", "reasoning": true, "efforts": ["custom"]}
        ]}
    }))
    .expect("model frame");
    let PiWireMessage::Response { response, .. } = parse_frame(&frame).expect("model response")
    else {
        panic!("expected response");
    };

    let Ok(Payload::ListModels(models)) = response.result else {
        panic!("expected models")
    };
    let efforts = models
        .iter()
        .map(|model| serde_json::json!(model.efforts))
        .collect::<Vec<_>>();
    assert_eq!(
        efforts,
        [
            serde_json::json!(["off"]),
            serde_json::json!(["off", "minimal", "low", "medium", "high"]),
            serde_json::json!(["off", "low", "medium", "high", "xhigh"]),
            serde_json::json!(["custom"]),
        ]
    );
}

#[test]
fn keeps_unknown_extension_methods_observable() {
    assert_eq!(
        parse_frame(br#"{"type":"extension_ui_request","id":"u1","method":"futureMethod"}"#),
        Ok(PiWireMessage::ExtensionUi(ExtensionUiRequest::Unknown {
            id: Some("u1".into()),
            method: "futureMethod".into(),
        }))
    );
}

#[test]
fn rejects_malformed_frames() {
    assert!(parse_frame(b"{").is_err());
    assert!(parse_frame(br#"{"command":"abort"}"#).is_err());
}

#[test]
fn malformed_catalogs_are_correlated_failures_not_empty_successes() {
    for (command, operation, bodies) in [
        (
            "get_available_models",
            SessionOperation::ListModels,
            vec![
                serde_json::json!({}),
                serde_json::json!({"models":null}),
                serde_json::json!({"models":[{"id":"incomplete"}]}),
                serde_json::json!({"models":[{"id":"valid","name":"Valid","provider":"test"},42]}),
            ],
        ),
        (
            "get_available_thinking_levels",
            SessionOperation::ListReasoningLevels,
            vec![
                serde_json::json!({}),
                serde_json::json!({"levels":["high",7]}),
            ],
        ),
        (
            "get_modes",
            SessionOperation::ListModes,
            vec![
                serde_json::json!({}),
                serde_json::json!({"modes":[{"id":"plan"}]}),
                serde_json::json!({"modes":[],"selected":3}),
            ],
        ),
        (
            "get_commands",
            SessionOperation::ListCommands,
            vec![
                serde_json::json!({}),
                serde_json::json!({"commands":[{"name":"bad","source":"unknown"}]}),
            ],
        ),
    ] {
        for data in bodies {
            let frame = serde_json::to_vec(&serde_json::json!({
                "type":"response", "id":"refresh", "command":command, "success":true, "data":data,
            }))
            .expect("wire fixture");
            let PiWireMessage::Response { response, .. } = parse_frame(&frame).expect("envelope")
            else {
                panic!("expected correlated response");
            };
            assert_eq!(response.id.as_deref(), Some("refresh"));
            assert_eq!(response.operation(), operation);
            assert!(response.result.is_err(), "{command}: {data}");
        }
    }
}

#[test]
fn empty_catalogs_are_valid_and_backend_rejections_keep_the_error() {
    for (command, data) in [
        ("get_available_models", serde_json::json!({"models":[]})),
        (
            "get_available_thinking_levels",
            serde_json::json!({"levels":[]}),
        ),
        ("get_modes", serde_json::json!({"modes":[]})),
        ("get_commands", serde_json::json!({"commands":[]})),
    ] {
        let frame = serde_json::to_vec(&serde_json::json!({
            "type":"response", "command":command, "success":true, "data":data,
        }))
        .expect("wire fixture");
        let PiWireMessage::Response { response, .. } = parse_frame(&frame).expect("envelope")
        else {
            panic!("response")
        };
        assert!(response.result.is_ok(), "{command}: {:?}", response.result);
    }
    let PiWireMessage::Response { response, .. } = parse_frame(
        br#"{"type":"response","id":"model","command":"set_model","success":false,"error":"provider unavailable","data":{}}"#
    ).expect("envelope") else { panic!("response") };
    assert_eq!(response.operation(), SessionOperation::SelectModel);
    assert_eq!(
        response.result.expect_err("rejected").message,
        "provider unavailable"
    );
}

#[test]
fn state_and_model_payloads_are_validated_before_leaving_pi() {
    for command in [
        "get_state",
        "set_model",
        "get_session_stats",
        "export_html",
        "fork",
    ] {
        let frame = serde_json::to_vec(&serde_json::json!({
            "type":"response", "command":command, "success":true, "data":{},
        }))
        .expect("wire fixture");
        let PiWireMessage::Response { response, .. } = parse_frame(&frame).expect("envelope")
        else {
            panic!("response")
        };
        assert!(response.result.is_err(), "{command}");
    }
}

#[test]
fn usage_preserves_cost_and_unknown_post_compaction_context() {
    let PiWireMessage::Response { response, .. } = parse_frame(
        br#"{"type":"response","command":"get_session_stats","success":true,"data":{"tokens":{"input":1,"output":2,"cacheRead":3,"cacheWrite":4,"total":10},"cost":0.25,"contextUsage":{"tokens":null,"contextWindow":1000,"percent":null}}}"#
    ).expect("usage frame") else { panic!("response") };
    let Ok(Payload::LoadUsage(usage)) = response.result else {
        panic!("usage")
    };
    assert_eq!(usage.tokens.total_tokens, 10);
    assert_eq!(usage.total_cost, Some(0.25));
    let context = usage.context_usage.expect("context");
    assert_eq!(context.tokens, None);
    assert_eq!(context.percent, None);
    assert_eq!(context.context_window, 1000);
}
