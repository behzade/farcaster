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
            let mut store = crate::app::persistence::open().expect("open state");
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
