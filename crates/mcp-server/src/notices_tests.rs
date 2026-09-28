use super::*;
use crate::agents::Backend;

fn params(value: serde_json::Value) -> Params {
    serde_json::from_value(value).expect("params")
}

fn caller(id: &str, name: &str) -> CallerContext {
    CallerContext {
        worker_id: id.into(),
        worker_name: name.into(),
        project: "/project".into(),
        session: format!("session-{id}"),
        session_locator: None,
        harness_profile_id: None,
        app_session_id: None,
        backend: Backend::Pi,
        provider: None,
        model: None,
        effort: None,
        access_mode: crate::agents::HarnessAccessMode::Auto,
        parent_worker_id: None,
    }
}

#[tokio::test]
async fn post_returns_other_relevant_notices_without_internal_ids() -> Result<(), String> {
    let board = NoticeBoard::default();
    let first = caller("internal-1", "OrangeCoyote");
    let second = caller("internal-2", "SilverHeron");
    board
        .access(
            &first,
            params(serde_json::json!({
                "action": "post", "message": "editing parser", "paths": ["src/parser"]
            })),
        )
        .await?;
    let response = board
        .access(
            &second,
            params(serde_json::json!({
                "action": "post", "message": "preparing parser commit", "paths": ["src/parser/mod.rs"]
            })),
        )
        .await?;

    assert!(response.posted);
    assert_eq!(response.notices[0].from, "OrangeCoyote");
    assert_eq!(response.notices[0].message, "editing parser");
    assert!(
        !serde_json::to_string(&response)
            .map_err(|error| error.to_string())?
            .contains("internal-1")
    );
    Ok(())
}

#[tokio::test]
async fn reads_can_filter_unrelated_paths() -> Result<(), String> {
    let board = NoticeBoard::default();
    let first = caller("one", "OrangeCoyote");
    let second = caller("two", "SilverHeron");
    board
        .access(
            &first,
            params(serde_json::json!({
                "action": "post", "message": "editing parser", "paths": ["src/parser.rs"]
            })),
        )
        .await?;
    let response = board
        .access(
            &second,
            params(serde_json::json!({"action": "read", "paths": ["src/ui"]})),
        )
        .await?;
    assert!(response.notices.is_empty());
    Ok(())
}

#[tokio::test]
async fn children_cannot_access_the_board() {
    let board = NoticeBoard::default();
    let mut child = caller("child", "review");
    child.parent_worker_id = Some("parent".into());
    assert!(
        board
            .access(&child, params(serde_json::json!({"action": "read"})))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn wait_returns_new_notices_then_a_bounded_timeout_with_a_reusable_cursor() {
    let board = NoticeBoard::default();
    let reader = caller("reader", "Reader");
    let writer = caller("writer", "Writer");
    board
        .access(
            &writer,
            params(serde_json::json!({"action": "post", "message": "old"})),
        )
        .await
        .expect("old post");
    let read = board
        .access(&reader, params(serde_json::json!({"action": "read"})))
        .await
        .expect("read");
    let posted = board
        .access(
            &writer,
            params(serde_json::json!({"action": "post", "message": "done"})),
        )
        .await
        .expect("post");
    assert!(posted.posted);
    assert!(!posted.timed_out);
    let response = board
        .access(
            &reader,
            params(serde_json::json!({"action": "wait", "after": read.cursor})),
        )
        .await
        .expect("wait");
    assert!(!response.posted);
    assert!(!response.timed_out);
    assert_eq!(response.notices.len(), 1);
    assert_eq!(response.notices[0].message, "done");
    assert_eq!(response.cursor, posted.cursor);
    let wait = board.access(
        &reader,
        params(
            serde_json::json!({"action": "wait", "after": response.cursor, "timeout_seconds": 1}),
        ),
    );
    let response = tokio::time::timeout(Duration::from_secs(2), wait)
        .await
        .expect("bounded wait")
        .expect("timeout response");
    assert!(response.timed_out);
    assert!(response.notices.is_empty());
    assert_eq!(response.cursor, posted.cursor);
    assert_eq!(
        serde_json::to_value(&response).expect("response")["timedOut"],
        true
    );
}

#[tokio::test]
async fn invalid_wait_options_fail_before_posting_or_waiting() {
    let board = NoticeBoard::default();
    let reader = caller("reader", "Reader");
    for value in [
        serde_json::json!({"action": "wait"}),
        serde_json::json!({"action": "wait", "after": "invalid"}),
        serde_json::json!({"action": "wait", "after": "cursor", "message": "unexpected"}),
        serde_json::json!({"action": "wait", "after": "cursor", "timeout_seconds": 0}),
        serde_json::json!({"action": "wait", "after": "cursor", "timeout_seconds": 61}),
        serde_json::json!({"action": "read", "after": "cursor"}),
        serde_json::json!({"action": "post", "message": "must not post", "timeout_seconds": 1}),
    ] {
        assert!(board.access(&reader, params(value)).await.is_err());
    }
    assert!(board.snapshot(&reader.project).is_empty());
}
