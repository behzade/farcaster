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
fn reads_renamed_sessions_from_the_captured_native_stream() -> Result<(), String> {
    // Captured verbatim from `opencode2 serve` (0.0.0-beta-19242) after
    // `POST /api/session/{id}/rename`; title changes use the same event.
    let input = b"data: {\"id\":\"evt_086002066001YblBJbai0du50R\",\"created\":1788954550374,\"type\":\"session.renamed\",\"location\":{\"directory\":\"/Users/behzad/Projects/personal/farcaster\"},\"data\":{\"sessionID\":\"ses_f7a01a4f4ffeN35QB351HErGTN\",\"title\":\"Second rename title\"},\"durable\":{\"aggregateID\":\"ses_f7a01a4f4ffeN35QB351HErGTN\",\"seq\":2,\"version\":1}}\n\n";
    let mut reader = BufReader::new(Cursor::new(input));

    let event = read_event(&mut reader)?.ok_or("expected event")?;
    assert_eq!(event.event.as_deref(), Some("session.renamed"));
    assert_eq!(event.data["sessionID"], "ses_f7a01a4f4ffeN35QB351HErGTN");
    assert_eq!(event.data["title"], "Second rename title");
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
