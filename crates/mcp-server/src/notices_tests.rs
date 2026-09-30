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
async fn reads_show_worker_names_and_hide_internal_ids() -> Result<(), String> {
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
    let read = board
        .access(
            &second,
            params(serde_json::json!({"action": "read", "paths": ["src/parser/mod.rs"]})),
        )
        .await?;
    assert_eq!(read.notices[0].from, "OrangeCoyote");
    assert_eq!(read.notices[0].message, "editing parser");
    assert!(read.cursor.is_some());
    assert!(
        !serde_json::to_string(&read)
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
    board
        .access(
            &writer,
            params(serde_json::json!({"action": "post", "message": "done"})),
        )
        .await
        .expect("post");
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
    assert!(response.cursor.is_some());
    let cursor = response.cursor.clone();
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
    assert_eq!(response.cursor, cursor);
    assert_eq!(
        serde_json::to_value(&response).expect("response")["timedOut"],
        true
    );
}

#[tokio::test]
async fn invalid_options_fail_before_posting_or_waiting() {
    let board = NoticeBoard::default();
    let reader = caller("reader", "Reader");
    for value in [
        serde_json::json!({"action": "wait"}),
        serde_json::json!({"action": "wait", "after": "invalid"}),
        serde_json::json!({"action": "wait", "after": "cursor", "message": "unexpected"}),
        serde_json::json!({"action": "wait", "after": "cursor", "timeout_seconds": 0}),
        serde_json::json!({"action": "wait", "after": "cursor", "timeout_seconds": 61}),
        serde_json::json!({"action": "read", "after": "invalid"}),
        serde_json::json!({"action": "read", "timeout_seconds": 1}),
        serde_json::json!({"action": "post", "message": "must not post", "after": "cursor"}),
        serde_json::json!({"action": "post", "message": "must not post", "timeout_seconds": 1}),
    ] {
        assert!(board.access(&reader, params(value)).await.is_err());
    }
    assert!(board.snapshot(&reader.project).is_empty());
}

#[tokio::test]
async fn cursor_reads_filter_new_notices_and_posts_do_not_skip_unread_messages()
-> Result<(), String> {
    let board = NoticeBoard::default();
    let reader = caller("reader", "Reader");
    let writer = caller("writer", "Writer");
    let read = board
        .access(
            &reader,
            params(serde_json::json!({"action": "read", "paths": ["src/parser"]})),
        )
        .await?;
    for (message, paths) in [
        ("parser work", vec!["src/parser/mod.rs"]),
        ("unrelated work", vec!["src/ui"]),
    ] {
        board
            .access(
                &writer,
                params(serde_json::json!({"action": "post", "message": message, "paths": paths})),
            )
            .await?;
    }
    let posted = board.access(&reader, params(serde_json::json!({"action": "post", "message": "own work", "paths": ["src/parser"]}))).await?;
    assert!(posted.posted);
    assert!(posted.notices.is_empty());
    assert!(
        serde_json::to_value(&posted)
            .map_err(|e| e.to_string())?
            .get("cursor")
            .is_none()
    );
    let fresh = board.access(&reader, params(serde_json::json!({"action": "read", "after": read.cursor, "paths": ["src/parser"]}))).await?;
    assert_eq!(fresh.notices.len(), 1);
    assert_eq!(fresh.notices[0].message, "parser work");
    let repeated = board.access(&reader, params(serde_json::json!({"action": "read", "after": fresh.cursor, "paths": ["src/parser"]}))).await?;
    assert!(repeated.notices.is_empty());
    assert!(!repeated.timed_out);
    assert_eq!(repeated.cursor, fresh.cursor);
    Ok(())
}
