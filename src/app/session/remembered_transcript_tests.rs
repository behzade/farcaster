use super::*;
use crate::conversation::ConversationState;
use serde_json::json;

fn snapshot(path: &Path, text: Option<&str>) -> Arc<RuntimeSnapshot> {
    let mut conversation = ConversationState::default();
    if let Some(text) = text {
        conversation.replace_history(&[json!({"role":"user", "content":text})]);
    }
    Arc::new(RuntimeSnapshot {
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
