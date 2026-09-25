use super::*;

#[test]
fn a_cached_history_is_served_without_reading_the_file_again() {
    let temp = tempfile::tempdir().expect("fixture");
    let path = temp.path().join("session.jsonl");
    let header = "{\"type\":\"session\",\"id\":\"root\",\"cwd\":\"/project\"}\n";
    let first = "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"first\"}}\n";
    let other = "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"other\"}}\n";
    std::fs::write(&path, format!("{header}{first}")).expect("fixture");
    let project = temp.path().to_path_buf();
    let before = load_cached_history(Backend::Pi, &path, &project).expect("load");

    let changed = format!("{header}{other}");
    assert_eq!(changed.len(), header.len() + first.len());
    let original = std::fs::metadata(&path)
        .expect("fixture")
        .modified()
        .expect("mtime");
    std::fs::write(&path, &changed).expect("fixture");
    let restored = std::fs::File::options()
        .write(true)
        .open(&path)
        .expect("fixture");
    restored.set_modified(original).expect("restore mtime");

    let after = load_cached_history(Backend::Pi, &path, &project).expect("load");
    assert_eq!(
        before.messages, after.messages,
        "a hit is served from memory even though the file now reads differently"
    );
    assert_ne!(
        after.messages,
        agents::load_session_history(Backend::Pi, &path, &project)
            .expect("load")
            .messages,
        "an uncached load would have seen the new contents"
    );
}

#[test]
fn a_grown_session_is_read_again() {
    let temp = tempfile::tempdir().expect("fixture");
    let path = temp.path().join("session.jsonl");
    let header = "{\"type\":\"session\",\"id\":\"root\",\"cwd\":\"/project\"}\n";
    let first = "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"first\"}}\n";
    let project = temp.path().to_path_buf();
    std::fs::write(&path, format!("{header}{first}")).expect("fixture");
    load_cached_history(Backend::Pi, &path, &project).expect("load");
    assert!(history_is_fresh(&path));

    std::fs::write(
        &path,
        format!(
            "{header}{first}{{\"type\":\"message\",\"message\":{{\"role\":\"assistant\",\"content\":\"second\"}}}}\n"
        ),
    )
    .expect("fixture");

    assert!(
        !history_is_fresh(&path),
        "a session that grew on disk must not be served from memory"
    );
    assert_eq!(
        load_cached_history(Backend::Pi, &path, &project)
            .expect("load")
            .messages
            .len(),
        2
    );
}
