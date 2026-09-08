use std::io::{BufReader, Cursor};

use serde_json::json;

use super::*;

#[test]
fn parses_native_data_only_envelope() -> Result<(), String> {
    let input = b"data: {\"id\":\"event-1\",\"type\":\"session.text.delta\",\"data\":{\"sessionID\":\"session-1\",\"delta\":\"hello\"}}\n\n";
    let mut reader = BufReader::new(Cursor::new(input));

    let event = read_event(&mut reader)?.ok_or("expected event")?;
    assert_eq!(event.id.as_deref(), Some("event-1"));
    assert_eq!(event.event.as_deref(), Some("session.text.delta"));
    assert_eq!(
        event.data,
        json!({"sessionID": "session-1", "delta": "hello"})
    );
    assert!(read_event(&mut reader)?.is_none());
    Ok(())
}

#[test]
fn preserves_unknown_events_and_multiline_data() -> Result<(), String> {
    let input = b": keepalive\nevent: future.event\ndata: first\ndata: second\n\n";
    let mut reader = BufReader::new(Cursor::new(input));

    let event = read_event(&mut reader)?.ok_or("expected event")?;
    assert_eq!(event.event.as_deref(), Some("future.event"));
    assert_eq!(event.data, Value::String("first\nsecond".into()));
    Ok(())
}
