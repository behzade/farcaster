use super::*;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Waker},
};

fn assert_pending(future: Pin<&mut impl Future>) {
    assert!(
        future
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
}

fn post(board: &NoticeBoard, project: &str, worker: &str, paths: &[&str]) {
    board
        .post(
            Path::new(project),
            worker.into(),
            worker.into(),
            "done".into(),
            paths.iter().map(PathBuf::from).collect(),
        )
        .expect("post");
}

#[test]
fn snapshots_are_newest_first_and_posts_emit_updates() -> Result<(), String> {
    let board = NoticeBoard::default();
    let updates = board.updates();
    for (id, name) in [("one", "OrangeCoyote"), ("two", "SilverHeron")] {
        board.post(
            Path::new("/project"),
            id.into(),
            name.into(),
            "editing shared files".into(),
            vec!["src".into()],
        )?;
    }

    assert!(updates.try_recv().is_ok());
    assert_eq!(
        board
            .snapshot(Path::new("/project"))
            .into_iter()
            .map(|notice| notice.from)
            .collect::<Vec<_>>(),
        ["SilverHeron", "OrangeCoyote"]
    );
    Ok(())
}

#[tokio::test]
async fn waiting_filters_own_posts_other_projects_and_unrelated_paths() {
    let board = NoticeBoard::default();
    let project = Path::new("/project");
    let paths = vec![PathBuf::from("src/parser")];
    let cursor = board
        .matching(project, "reader", &paths, None)
        .expect("read")
        .cursor;
    let mut waiting =
        Box::pin(board.wait(project, "reader", &paths, &cursor, Duration::from_secs(1)));
    assert_pending(waiting.as_mut());
    for (project, worker, paths) in [
        ("/project", "reader", vec![]),
        ("/other", "writer", vec![]),
        ("/project", "writer", vec!["src/parsers"]),
    ] {
        post(&board, project, worker, &paths);
        assert_pending(waiting.as_mut());
    }
    post(&board, "/project", "writer", &["src/parser/file.rs"]);
    let batch = waiting.await.expect("matching notice");
    assert_eq!(batch.notices.len(), 1);
    assert_eq!(batch.notices[0].paths, ["src/parser/file.rs"]);
}

#[tokio::test]
async fn a_project_wide_notice_wakes_every_waiter_and_preserves_ui_updates() {
    let board = NoticeBoard::default();
    let project = Path::new("/project");
    let cursor = board
        .matching(project, "reader", &[], None)
        .expect("read")
        .cursor;
    let paths = vec![PathBuf::from("src")];
    let first = board.wait(project, "one", &paths, &cursor, Duration::from_secs(1));
    let second = board.wait(project, "two", &paths, &cursor, Duration::from_secs(1));
    let publish = async {
        tokio::task::yield_now().await;
        post(&board, "/project", "writer", &[]);
    };
    let (first, second, ()) = tokio::time::timeout(Duration::from_millis(500), async {
        tokio::join!(first, second, publish)
    })
    .await
    .expect("both waiters wake promptly");
    assert_eq!(first.expect("first").notices.len(), 1);
    assert_eq!(second.expect("second").notices.len(), 1);
    assert!(board.updates().try_recv().is_ok());
}

#[tokio::test]
async fn dropping_a_wait_unsubscribes_without_consuming_notices() {
    let board = NoticeBoard::default();
    let project = Path::new("/project");
    let cursor = board
        .matching(project, "reader", &[], None)
        .expect("read")
        .cursor;
    let mut waiting = Box::pin(board.wait(project, "reader", &[], &cursor, Duration::from_secs(1)));
    assert_pending(waiting.as_mut());
    assert_eq!(board.changes.receiver_count(), 1);
    drop(waiting);
    assert_eq!(board.changes.receiver_count(), 0);
    post(&board, "/project", "writer", &[]);
    assert_eq!(
        board
            .wait(project, "reader", &[], &cursor, Duration::ZERO)
            .await
            .expect("retry")
            .notices
            .len(),
        1
    );
}

#[test]
fn cursors_reject_wrong_projects_restarts_and_missing_history() {
    let board = NoticeBoard::default();
    let project = Path::new("/project");
    let cursor = board
        .matching(project, "reader", &[], None)
        .expect("read")
        .cursor;
    assert!(
        board
            .matching(Path::new("/other"), "reader", &[], Some(&cursor))
            .is_err()
    );
    assert!(
        NoticeBoard::default()
            .matching(project, "reader", &[], Some(&cursor))
            .is_err()
    );
    assert!(
        board
            .matching(project, "reader", &[], Some("invalid"))
            .is_err()
    );
    let future_cursor = format!("{}:1", cursor.split_once(':').expect("cursor").0);
    assert!(
        board
            .matching(project, "reader", &[], Some(&future_cursor))
            .is_err()
    );
    for _ in 0..=MAX_PROJECT_NOTICES {
        post(&board, "/project", "writer", &[]);
    }
    assert!(
        board
            .matching(project, "reader", &[], Some(&cursor))
            .is_err()
    );
    assert_eq!(board.snapshot(project).len(), MAX_PROJECT_NOTICES);
}

#[test]
fn expired_unread_notices_require_a_fresh_read() {
    let board = NoticeBoard::default();
    let project = Path::new("/project");
    let cursor = board
        .matching(project, "reader", &[], None)
        .expect("read")
        .cursor;
    post(&board, "/project", "writer", &[]);
    let read_cursor = board
        .matching(project, "reader", &[], None)
        .expect("read")
        .cursor;
    board
        .entries
        .lock()
        .expect("board")
        .get_mut(project)
        .expect("project")
        .notices[0]
        .created_at -= NOTICE_TTL;
    assert!(
        board
            .matching(project, "reader", &[], Some(&cursor))
            .is_err()
    );
    assert!(
        board
            .matching(project, "reader", &[], Some(&read_cursor))
            .expect("already read history")
            .notices
            .is_empty()
    );
}
