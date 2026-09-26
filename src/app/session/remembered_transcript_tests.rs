use super::*;
use crate::conversation::ConversationState;
use serde_json::json;

fn snapshot(path: &Path, text: Option<&str>) -> Arc<RuntimeSnapshot> {
    let mut conversation = ConversationState::default();
    if let Some(text) = text {
        conversation.replace_history(&[json!({"role":"user", "content":text})]);
    }
    Arc::new(RuntimeSnapshot {
        status: if text.is_some() {
            "Ready"
        } else {
            "Loading history"
        }
        .into(),
        selected_session: Some(path.to_path_buf()),
        conversation: Arc::new(conversation),
        ..Default::default()
    })
}

fn stamped_session(directory: &Path, contents: &str) -> PathBuf {
    let path = directory.join("chat.jsonl");
    std::fs::write(&path, contents).expect("write session file");
    path
}

#[test]
fn a_loading_snapshot_paints_the_session_it_last_showed() {
    let directory = tempfile::tempdir().expect("session directory");
    let path = stamped_session(directory.path(), "{}");

    remember(&snapshot(&path, Some("hello")));

    let loading = stand_in(snapshot(&path, None));
    assert_eq!(loading.conversation.items.len(), 1);
}

#[test]
fn an_empty_session_is_never_remembered() {
    let directory = tempfile::tempdir().expect("session directory");
    let path = stamped_session(directory.path(), "{}");

    remember(&snapshot(&path, None));

    assert!(
        stand_in(snapshot(&path, None))
            .conversation
            .items
            .is_empty()
    );
}

#[test]
fn another_session_never_borrows_what_this_one_showed() {
    let directory = tempfile::tempdir().expect("session directory");
    let remembered = stamped_session(directory.path(), "{}");
    let other = directory.path().join("other.jsonl");
    std::fs::write(&other, "{}").expect("write other session file");

    remember(&snapshot(&remembered, Some("hello")));

    assert!(
        stand_in(snapshot(&other, None))
            .conversation
            .items
            .is_empty()
    );
}

#[test]
fn a_session_that_moved_on_is_not_served_from_memory() {
    let directory = tempfile::tempdir().expect("session directory");
    let path = stamped_session(directory.path(), "{}");
    remember(&snapshot(&path, Some("hello")));

    std::fs::write(&path, "{\"grew\": true}").expect("grow session file");

    assert!(
        stand_in(snapshot(&path, None))
            .conversation
            .items
            .is_empty()
    );
}

#[test]
fn loaded_content_is_left_alone() {
    let directory = tempfile::tempdir().expect("session directory");
    let path = stamped_session(directory.path(), "{}");
    remember(&snapshot(&path, Some("hello")));

    let loaded = stand_in(snapshot(&path, Some("fresh")));
    assert_eq!(loaded.conversation.items.len(), 1);
}

#[test]
fn a_finished_empty_or_failed_load_clears_the_old_transcript() {
    let directory = tempfile::tempdir().expect("session directory");
    let path = stamped_session(directory.path(), "{}");
    for status in ["Ready", "Could not load history"] {
        remember(&snapshot(&path, Some("old")));
        let mut finished = snapshot(&path, None);
        Arc::make_mut(&mut finished).status = status.into();
        remember(&finished);
        assert!(Arc::ptr_eq(&stand_in(finished.clone()), &finished));
        assert!(
            stand_in(snapshot(&path, None))
                .conversation
                .items
                .is_empty()
        );
    }
}

#[test]
fn a_loading_snapshot_does_not_replace_the_remembered_read() {
    let directory = tempfile::tempdir().expect("session directory");
    let path = stamped_session(directory.path(), "{}");
    remember(&snapshot(&path, Some("old")));
    let loading = snapshot(&path, None);
    remember(&loading);
    let shown = stand_in(loading);
    assert_eq!(shown.conversation.items.len(), 1);
    assert_eq!(shown.status, "Loading history");
    assert_eq!(shown.transcript_changed_from, Some(0));
}

#[test]
fn a_different_project_backend_or_profile_cannot_reuse_the_transcript() {
    let directory = tempfile::tempdir().expect("session directory");
    let path = stamped_session(directory.path(), "{}");
    remember(&snapshot(&path, Some("old")));
    for context in 0..3 {
        let mut loading = snapshot(&path, None);
        let state = Arc::make_mut(&mut loading);
        match context {
            0 => state.project = directory.path().join("other-project"),
            1 => state.harness = Some(crate::agents::Backend::Claude),
            _ => state.profile_id = Some("other-profile".into()),
        }
        assert!(Arc::ptr_eq(&stand_in(loading.clone()), &loading));
    }
}
