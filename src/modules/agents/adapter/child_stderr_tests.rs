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
