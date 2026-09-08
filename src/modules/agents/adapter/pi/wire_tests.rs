use super::*;

#[test]
fn parses_response_and_activity_frames() {
    assert_eq!(
        parse_frame(br#"{"type":"response","id":"1","command":"abort","success":true}"#),
        Ok(PiWireMessage::Response {
            command: "abort".into(),
            response: SessionResponse {
                id: Some("1".into()),
                operation: SessionOperation::Abort,
                success: true,
                data: Value::Null,
                error: None,
            },
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
    assert_eq!(response.data["tokens"]["totalTokens"], 8);
}

#[test]
fn history_response_projects_pi_entries_before_leaving_adapter() {
    let PiWireMessage::Response { response, .. } = parse_frame(
            br#"{"type":"response","command":"get_entries","success":true,"data":{"entries":[{"type":"message","id":"a","parentId":null,"message":{"role":"user","content":"hello"}}]}}"#,
        ).expect("history") else { panic!("expected response"); };
    assert_eq!(response.data["messages"][0]["content"], "hello");
    assert!(response.data.get("entries").is_none());
    assert!(
        parse_frame(br#"{"type":"response","command":"get_entries","success":true,"data":{}}"#)
            .is_err()
    );
}

#[test]
fn adds_per_model_efforts_to_pi_model_catalogs() {
    let frame = serde_json::to_vec(&serde_json::json!({
        "type": "response",
        "command": "get_available_models",
        "success": true,
        "data": {"models": [
            {"id": "plain", "reasoning": false},
            {"id": "default", "reasoning": true},
            {"id": "mapped", "reasoning": true, "thinkingLevelMap": {
                "minimal": null,
                "xhigh": "xhigh",
                "max": null
            }},
            {"id": "future", "reasoning": true, "efforts": ["custom"]}
        ]}
    }))
    .expect("model frame");
    let PiWireMessage::Response { response, .. } = parse_frame(&frame).expect("model response")
    else {
        panic!("expected response");
    };

    let models = response.data["models"].as_array().expect("models");
    let efforts = models
        .iter()
        .map(|model| model["efforts"].clone())
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
