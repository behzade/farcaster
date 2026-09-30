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
fn envelope_extraction_keeps_sse_precedence_and_unrecognized_envelopes() {
    for data in [json!({"large":"payload"}), Value::Null] {
        let input = format!(
            "id: sse-id\nevent: sse-event\ndata: {}\n\n",
            json!({"id":"native-id", "type":"native-type", "data":data})
        );
        let event = read_event(&mut Cursor::new(input)).unwrap().unwrap();
        assert_eq!(event.id.as_deref(), Some("sse-id"));
        assert_eq!(event.event.as_deref(), Some("sse-event"));
        assert_eq!(event.data, data);
    }
    for envelope in [
        json!({"type":"native-type"}),
        json!({"type":42, "data":{"keep":"nested"}}),
        json!({"data":null}),
        json!([1, 2]),
    ] {
        let input = format!("data: {envelope}\n\n");
        let event = read_event(&mut Cursor::new(input)).unwrap().unwrap();
        assert!(event.id.is_none());
        assert!(event.event.is_none());
        assert_eq!(event.data, envelope);
    }
}

#[test]
fn reads_renamed_sessions_from_the_captured_native_stream() -> Result<(), String> {
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

#[test]
fn classifies_versioned_aliases_and_unknown_events() {
    for (native, expected) in [
        ("catalog.updated", OpenCodeEventKind::CatalogUpdated),
        ("model.updated", OpenCodeEventKind::CatalogUpdated),
        ("session.next.text.delta", OpenCodeEventKind::TextDelta),
        (
            "session.next.reasoning.ended",
            OpenCodeEventKind::ReasoningEnded,
        ),
        (
            "session.next.tool.input.started",
            OpenCodeEventKind::ToolInputStarted,
        ),
        ("session.next.tool.failed", OpenCodeEventKind::ToolFailed),
        ("session.next.step.ended.2", OpenCodeEventKind::StepEnded),
        ("permission.asked.1", OpenCodeEventKind::PermissionAsked),
        (
            "session.next.compaction.started",
            OpenCodeEventKind::CompactionStarted,
        ),
    ] {
        assert_eq!(OpenCodeEventKind::parse(native), expected);
    }
    assert_eq!(
        OpenCodeEventKind::parse("future.event"),
        OpenCodeEventKind::Unknown("future.event")
    );
}

#[test]
fn event_kinds_own_scope_and_execution_classification() {
    assert!(OpenCodeEventKind::TextDelta.is_execution());
    assert!(OpenCodeEventKind::parse("session.next.future.delta").is_session_scoped());
    assert!(OpenCodeEventKind::parse("session.next.text.future").is_execution());
    assert!(!OpenCodeEventKind::CatalogUpdated.is_session_scoped());
}
