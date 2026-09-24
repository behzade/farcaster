use super::*;

use std::time::SystemTime;

fn remembered_session(project: &Path, id: &str, title: &str) -> SessionSummary {
    SessionSummary::from_cached(
        id.into(),
        project.join(format!("{id}.jsonl")),
        project.to_path_buf(),
        title.into(),
        "first prompt".into(),
        "2026-09-24T00:00:00Z".into(),
        None,
        SystemTime::now(),
        3,
        crate::sessions::UsageSummary::default(),
        false,
        false,
        title.into(),
    )
}

#[gpui::test]
fn the_rail_paints_stored_chats_before_the_runtime_answers(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_prepared_offline_app(
        concat!(
            module_path!(),
            "::the_rail_paints_stored_chats_before_the_runtime_answers"
        ),
        cx,
        |project| {
            let session = remembered_session(project, "remembered", "Remembered chat");
            std::fs::write(&session.path, "{}").expect("write session file");
            let mut store =
                crate::app::infrastructure::persistence::StateStore::open().expect("open state");
            store
                .replace_sessions(&[session])
                .expect("index the stored session");
        },
        |cx, app, _, _| {
            cx.update(|_, cx| {
                let app = app.read(cx);
                assert_eq!(
                    app.sessions.all.len(),
                    1,
                    "rail catalog came from the store"
                );
                assert_eq!(app.sessions.visible.len(), 1);
                assert_eq!(app.sessions.all[0].title, "Remembered chat");
            });
        },
    );
}

#[gpui::test]
fn a_launch_warms_the_history_of_the_chat_most_likely_to_open_next(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_prepared_offline_app(
        concat!(
            module_path!(),
            "::a_launch_warms_the_history_of_the_chat_most_likely_to_open_next"
        ),
        cx,
        |project| {
            std::fs::write(
                project.join("recent.jsonl"),
                concat!(
                    r#"{"type":"session","version":3,"id":"recent","cwd":"/project"}"#,
                    "\n",
                    r#"{"type":"message","id":"one","message":{"role":"user","content":"hello"}}"#,
                    "\n",
                ),
            )
            .expect("write session file");
            let session = remembered_session(project, "recent", "Recent chat");
            let mut store =
                crate::app::infrastructure::persistence::StateStore::open().expect("open state");
            store
                .replace_sessions(&[session])
                .expect("index the stored session");
        },
        |_, _, _, project| {
            let path = crate::sessions::normalize_session_path(&project.join("recent.jsonl"));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !crate::app::runtime::history_cache::history_is_fresh(&path)
                && std::time::Instant::now() < deadline
            {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert!(
                crate::app::runtime::history_cache::history_is_fresh(&path),
                "a launch should load the most recently written chat's history",
            );
        },
    );
}
