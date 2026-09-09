use super::{Level, structured_level};

#[test]
fn structured_stderr_uses_embedded_level() {
    assert_eq!(
        structured_level(
            r#"timestamp=2026-09-04T08:22:46.865Z level=INFO message="spawning process""#
        ),
        Some(Level::Info)
    );
    assert_eq!(
        structured_level(r#"{"level":"error","msg":"boom"}"#),
        Some(Level::Error)
    );
    assert_eq!(structured_level("not a structured log line"), None);
}

#[test]
fn glog_stderr_preserves_severity_and_demotes_raw_payloads() {
    for (prefix, message, expected) in [
        ("I", "Starting AGY ACP Server...", Level::Info),
        ("I", "RAW WS MSG: {\"stepUpdate\":{}}", Level::Debug),
        ("W", "RAW WS MSG: failed", Level::Warn),
        ("E", "connection failed", Level::Error),
        ("F", "fatal failure", Level::Error),
        ("I", "payload level=ERROR", Level::Info),
    ] {
        let line =
            format!("{prefix}0909 17:52:19.526654 8372150400 local_connection.py:521] {message}");
        assert_eq!(structured_level(&line), Some(expected), "{line}");
    }
}

#[test]
fn ordinary_stderr_is_not_mistaken_for_glog() {
    for line in [
        "I0909 bad-time 123 main.py:80] message",
        "I0909 17:52:19.526654 thread main.py:80] message",
        "I0909 17:52:19.526654 123 main.py:] message",
        "I0909 17:52:19.526654 123 main.py:80 extra] message",
        "RAW WS MSG: plain stderr must still warn",
    ] {
        assert_eq!(structured_level(line), None, "{line}");
    }
}
