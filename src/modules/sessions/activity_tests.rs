use super::*;

#[test]
fn parses_rfc3339_session_timestamps() {
    assert_eq!(
        parse_iso_timestamp("1970-01-01T00:00:01.250Z")
            .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok()),
        Some(Duration::from_millis(1_250))
    );
    assert_eq!(
        parse_iso_timestamp("1970-01-01T01:00:01.250+01:00")
            .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok()),
        Some(Duration::from_millis(1_250))
    );
    assert_eq!(parse_iso_timestamp("2025-02-29T00:00:00Z"), None);
    assert_eq!(parse_iso_timestamp("not-a-timestamp"), None);
}

#[test]
fn pairs_current_and_recent_tools() {
    let mut builder = ActivityBuilder::default();
    builder.observe_entry(&serde_json::json!({
        "type":"message",
        "message":{"role":"assistant","stopReason":"toolUse","content":[
            {"type":"toolCall","id":"one","name":"edit","arguments":{"path":"src/main.rs"}}
        ]}
    }));
    let active = builder.finish(
        "child".into(),
        PathBuf::from("/sessions/child"),
        "Implementation session",
        "Implement the feature",
        UsageSummary::default(),
        SystemTime::UNIX_EPOCH,
        SystemTime::UNIX_EPOCH + Duration::from_secs(4),
        true,
        false,
    );
    assert_eq!(active.lifecycle, AgentLifecycle::Working);
    assert_eq!(
        active.current_tool.as_ref().map(|tool| tool.name.as_str()),
        Some("edit")
    );
    let mut builder = ActivityBuilder::default();
    builder.observe_entry(&serde_json::json!({
        "type":"message",
        "message":{"role":"assistant","stopReason":"toolUse","content":[
            {"type":"toolCall","id":"one","name":"read","arguments":{"path":"README.md"}}
        ]}
    }));
    builder.observe_entry(&serde_json::json!({
            "type":"message","message":{"role":"toolResult","toolCallId":"one","toolName":"read","isError":false}
        }));
    builder.observe_entry(&serde_json::json!({
        "type":"message","message":{"role":"assistant","stopReason":"stop","content":[]}
    }));
    let done = builder.finish(
        "child".into(),
        PathBuf::from("/sessions/child"),
        "Review session",
        "Review",
        UsageSummary::default(),
        SystemTime::UNIX_EPOCH,
        SystemTime::UNIX_EPOCH + Duration::from_secs(4),
        false,
        false,
    );
    assert_eq!(
        done.lifecycle,
        AgentLifecycle::Completed(AgentOutcome::Complete)
    );
    assert!(done.current_tool.is_none());
    assert_eq!(
        done.recent_tool.as_ref().map(|tool| tool.name.as_str()),
        Some("read")
    );
    assert_eq!(done.elapsed, Some(Duration::from_secs(4)));
}

#[test]
fn out_of_order_parallel_results_leave_an_outstanding_current_tool() {
    let mut builder = ActivityBuilder::default();
    builder.observe_entry(&serde_json::json!({
        "type":"message","message":{"role":"assistant","stopReason":"toolUse","content":[
            {"type":"toolCall","id":"one","name":"read","arguments":{"path":"one"}},
            {"type":"toolCall","id":"two","name":"read","arguments":{"path":"two"}}
        ]}
    }));
    builder.observe_entry(&serde_json::json!({
            "type":"message","message":{"role":"toolResult","toolCallId":"two","toolName":"read","isError":false}
        }));
    let activity = builder.finish(
        "id".into(),
        PathBuf::new(),
        "worker",
        "task",
        UsageSummary::default(),
        SystemTime::UNIX_EPOCH,
        SystemTime::UNIX_EPOCH,
        true,
        false,
    );
    assert_eq!(
        activity
            .current_tool
            .as_ref()
            .map(|tool| tool.target.as_str()),
        Some("one")
    );
}

#[test]
fn terminal_outcomes_use_only_explicit_stop_reasons() {
    for (reason, expected) in [
        ("error", AgentOutcome::Failed),
        ("aborted", AgentOutcome::Incomplete),
        ("length", AgentOutcome::Incomplete),
    ] {
        let mut builder = ActivityBuilder::default();
        builder.observe_entry(&serde_json::json!({
            "type":"message","message":{"role":"assistant","stopReason":reason,"content":[]}
        }));
        let activity = builder.finish(
            "id".into(),
            PathBuf::new(),
            "worker",
            "task",
            UsageSummary::default(),
            SystemTime::UNIX_EPOCH,
            SystemTime::UNIX_EPOCH,
            false,
            false,
        );
        assert_eq!(activity.lifecycle, AgentLifecycle::Completed(expected));
    }
}
