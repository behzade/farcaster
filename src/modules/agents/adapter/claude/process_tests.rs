use super::*;

#[test]
fn native_queue_status_does_not_bypass_validation_for_other_frames() {
    assert!(
        decode_frame(r#"{"type":"command_lifecycle"}"#)
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        decode_frame(r#"{"type":"keep_alive"}"#).unwrap(),
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
