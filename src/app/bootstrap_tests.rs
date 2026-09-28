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

#[test]
fn warming_uses_the_sessions_profile_and_rejects_a_removed_profile() {
    let directory = tempfile::tempdir().expect("project");
    let project = directory.path();
    let profile = crate::agents::HarnessProfile {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Warm history".into(),
        backend: Backend::Pi,
        executable: PathBuf::from("pi"),
        data_directory: None,
    };
    let config = crate::agents::AgentLaunchConfig::default();
    config
        .profiles
        .replace(vec![profile.clone()])
        .expect("profile");
    let mut session = remembered_session(project, "recent", "Recent chat");
    let path = project.join("recent.jsonl");
    session.profile_id = Some(profile.id.clone());
    std::fs::write(
        &path,
        concat!(
            "{\"type\":\"session\",\"id\":\"recent\",\"cwd\":\"/project\"}\n",
            "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n",
        ),
    )
    .expect("history");
    session.path = path;
    let sessions = [session];
    let warmed = warm_recent_history(&sessions, project, config.clone())
        .expect("warm task")
        .join()
        .expect("warm thread")
        .expect("profile history");
    assert!(warmed);
    config.profiles.replace(vec![]).expect("remove profile");
    let error = warm_recent_history(&sessions, project, config)
        .expect("warm task")
        .join()
        .expect("warm thread")
        .expect_err("removed profile");
    assert!(error.contains("unknown harness profile"), "{error}");
}

#[test]
fn warming_uses_catalog_roots_for_app_children_and_orphans() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path();
    let mut root = remembered_session(project, "root", "Root");
    root.app_session_id = 1;
    std::fs::write(
        &root.path,
        "{\"type\":\"session\",\"id\":\"root\",\"cwd\":\"/project\"}\n",
    )
    .unwrap();
    let mut child = remembered_session(project, "child", "Child");
    child.app_session_id = 2;
    child.parent_app_session_id = Some(root.app_session_id);
    child.modified = root.modified + std::time::Duration::from_secs(1);
    let sessions = [root.clone(), child];
    assert!(
        warm_recent_history(&sessions, project, Default::default())
            .unwrap()
            .join()
            .unwrap()
            .unwrap()
    );

    root.parent_session = Some("missing-parent".into());
    assert!(
        warm_recent_history(&[root], project, Default::default())
            .expect("an orphan remains a visible catalog root")
            .join()
            .unwrap()
            .unwrap()
    );
}
