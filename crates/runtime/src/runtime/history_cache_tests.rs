use super::*;

fn history() -> LoadedHistory {
    LoadedHistory {
        messages: vec![Value::from("cached")],
        model: None,
        thinking_level: None,
        pending_question: None,
        prompt_deliveries: None,
    }
}

fn a_stamp(len: u64) -> Stamp {
    Stamp {
        modified: None,
        len,
    }
}

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
fn a_history_is_kept_until_the_file_changes() {
    let mut cache = HistoryCache {
        entries: Vec::new(),
    };
    let path = PathBuf::from("/chats/one.jsonl");
    cache.store(path.clone(), Some(a_stamp(12)), &history());
    assert_eq!(
        cache
            .take(&path, Some(&a_stamp(12)))
            .expect("cached")
            .messages
            .len(),
        1
    );
    assert!(
        cache.take(&path, Some(&a_stamp(13))).is_none(),
        "a session that grew on disk must be read again"
    );
}

#[test]
fn a_file_that_cannot_be_stamped_is_never_reused() {
    let mut cache = HistoryCache {
        entries: Vec::new(),
    };
    let path = PathBuf::from("/chats/gone.jsonl");
    cache.store(path.clone(), None, &history());
    assert!(cache.take(&path, None).is_none());
    assert!(cache.entries.is_empty());
}

#[test]
fn the_oldest_history_is_dropped_once_the_cache_is_full() {
    let mut cache = HistoryCache {
        entries: Vec::new(),
    };
    for index in 0..=LIMIT {
        cache.store(
            PathBuf::from(format!("/chats/{index}.jsonl")),
            Some(a_stamp(1)),
            &history(),
        );
    }
    assert_eq!(cache.entries.len(), LIMIT);
    assert!(
        cache
            .take(&PathBuf::from("/chats/0.jsonl"), Some(&a_stamp(1)))
            .is_none(),
        "the least recently used history is the one that goes"
    );
    assert!(
        cache
            .take(&PathBuf::from("/chats/1.jsonl"), Some(&a_stamp(1)))
            .is_some()
    );
}

#[test]
fn a_hit_counts_as_a_use() {
    let mut cache = HistoryCache {
        entries: Vec::new(),
    };
    let first = PathBuf::from("/chats/first.jsonl");
    cache.store(first.clone(), Some(a_stamp(1)), &history());
    for index in 0..LIMIT - 1 {
        cache.store(
            PathBuf::from(format!("/chats/other-{index}.jsonl")),
            Some(a_stamp(1)),
            &history(),
        );
        assert!(cache.take(&first, Some(&a_stamp(1))).is_some());
    }
    assert!(
        cache.take(&first, Some(&a_stamp(1))).is_some(),
        "a history read again and again stays cached"
    );
}
