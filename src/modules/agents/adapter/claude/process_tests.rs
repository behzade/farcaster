use super::*;

#[test]
fn real_partial_thinking_block_does_not_require_a_future_signature() {
    let frame = json!({"type":"stream_event","event":{"type":"content_block_start",
        "content_block":{"type":"thinking","thinking":""},"index":0},
        "parent_tool_use_id":null,"session_id":"fixture","uuid":"fixture"});
    assert_eq!(
        serde_json::to_value(
            decode_frame(&frame.to_string())
                .expect("test operation should succeed")
                .expect("test operation should succeed")
        )
        .expect("test operation should succeed"),
        frame
    );
}

#[test]
fn real_cli_stream_decodes_without_inventing_missing_metadata() {
    for line in include_str!("fixtures/cli-2.1.236.jsonl").lines() {
        let original: Value = serde_json::from_str(line).expect("test operation should succeed");
        let decoded = decode_frame(line)
            .expect("test operation should succeed")
            .expect("test operation should succeed");
        assert_eq!(
            serde_json::to_value(decoded).expect("test operation should succeed"),
            original
        );
    }
    let mut frame: Value = serde_json::from_str(
        include_str!("fixtures/cli-2.1.236.jsonl")
            .lines()
            .next()
            .expect("test operation should succeed"),
    )
    .expect("test operation should succeed");
    frame["event"]["message"]["usage"]["input_tokens"] = json!("wrong type");
    assert!(decode_frame(&frame.to_string()).is_err());
}

#[test]
fn native_queue_status_does_not_bypass_validation_for_other_frames() {
    assert!(
        decode_frame(r#"{"type":"command_lifecycle"}"#)
            .expect("test operation should succeed")
            .is_none()
    );
    assert!(matches!(
        decode_frame(r#"{"type":"keep_alive"}"#).expect("test operation should succeed"),
        Some(StdoutMessage::SDKKeepAliveMessage(_))
    ));
    for line in [
        r#"{"type":"result","subtype":"success"}"#,
        r#"{"type":"assistant","message":{}}"#,
        r#"{"type":"control_request","request_id":"permission","request":{}}"#,
        r#"{"type":"command_lifecycle""#,
        r#"{"type":"unknown_frame"}"#,
    ] {
        assert!(decode_frame(line).is_err(), "must reject {line}");
    }
}
