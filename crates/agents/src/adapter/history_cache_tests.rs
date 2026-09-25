use super::*;
use farcaster_sessions::LoadedHistory;
use std::{cell::Cell, io::Write as _, path::PathBuf, sync::mpsc};

fn history(text: &str) -> LoadedHistory {
    LoadedHistory {
        messages: vec![serde_json::Value::from(text)],
        model: None,
        thinking_level: None,
        pending_question: None,
        prompt_deliveries: None,
    }
}

type FileCache = HistoryCache<PathBuf, FileStamp, LoadedHistory>;

fn load_with(
    cache: &FileCache,
    path: &Path,
    load: impl FnOnce(&Path) -> Result<LoadedHistory, String>,
) -> Result<LoadedHistory, String> {
    cache.load(path.to_path_buf(), || FileStamp::read(path), || load(path))
}

fn parse(path: &Path) -> Result<LoadedHistory, String> {
    super::super::pi::session_files::load_history(path)
}

fn fixture() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().expect("fixture");
    let path = temp.path().join("session.jsonl");
    std::fs::write(
        &path,
        concat!(
            "{\"type\":\"session\",\"id\":\"root\",\"cwd\":\"/project\"}\n",
            "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"first\"}}\n"
        ),
    )
    .expect("fixture");
    (temp, path)
}

fn append(path: &Path) {
    let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(
        file,
        "{}",
        serde_json::json!({"type": "message", "message": {"role": "user", "content": "second"}})
    )
    .unwrap();
}

#[test]
fn repeated_loads_skip_parsing_until_the_file_changes() {
    let (_temp, path) = fixture();
    let cache = FileCache::new();
    let parses = Cell::new(0);
    let counted = |path: &Path| {
        parses.set(parses.get() + 1);
        parse(path)
    };
    let first = load_with(&cache, &path, counted).unwrap();
    let mut hit = load_with(&cache, &path, counted).unwrap();
    assert_eq!(first.messages, hit.messages);
    assert_eq!(parses.get(), 1);
    hit.messages.clear();
    assert_eq!(
        load_with(&cache, &path, counted).unwrap().messages,
        first.messages
    );
    assert_eq!(
        parses.get(),
        1,
        "consumer annotations cannot mutate cached data"
    );
    append(&path);
    let updated = load_with(&cache, &path, counted).unwrap();
    assert_eq!(parses.get(), 2);
    assert_eq!(updated.messages.len(), first.messages.len() + 1);
    assert_eq!(updated.messages.last().unwrap()["content"], "second");
}

#[test]
fn a_timestamp_change_invalidates_even_when_size_is_unchanged() {
    let (_temp, path) = fixture();
    let cache = FileCache::new();
    load_with(&cache, &path, |_| Ok(history("first"))).unwrap();
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified + std::time::Duration::from_secs(2))
        .unwrap();
    let loaded = load_with(&cache, &path, |_| Ok(history("updated"))).unwrap();
    assert_eq!(loaded.messages, history("updated").messages);
}

#[test]
fn deleted_or_non_file_history_is_not_reused() {
    let (_temp, path) = fixture();
    let cache = FileCache::new();
    load_with(&cache, &path, parse).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(load_with(&cache, &path, parse).is_err());
    std::fs::create_dir(&path).unwrap();
    assert!(FileStamp::read(&path).is_none());
    assert!(FileStamp::read(Path::new("session.jsonl")).is_none());
}

#[test]
fn failed_load_is_retried_instead_of_cached() {
    let (_temp, path) = fixture();
    let cache = FileCache::new();
    assert!(load_with(&cache, &path, |_| Err("read failed".into())).is_err());
    let loaded = load_with(&cache, &path, parse).unwrap();
    assert_eq!(loaded.messages.len(), 1);
}

#[test]
fn a_file_changed_during_parsing_is_not_cached() {
    let (_temp, path) = fixture();
    let cache = FileCache::new();
    let old = load_with(&cache, &path, |path| {
        let history = parse(path)?;
        append(path);
        Ok(history)
    })
    .unwrap();
    assert_eq!(old.messages.len(), 1);
    let new = load_with(&cache, &path, parse).unwrap();
    assert_eq!(new.messages.len(), 2);
}

#[test]
fn a_delayed_old_load_cannot_replace_a_newer_fill() {
    let (_temp, path) = fixture();
    let cache = FileCache::new();
    let (read_tx, read_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        let cache_ref = &cache;
        let path_ref = &path;
        let old = scope.spawn(move || {
            load_with(cache_ref, path_ref, |path| {
                let history = parse(path)?;
                read_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(history)
            })
        });
        read_rx.recv().unwrap();
        append(&path);
        let new = load_with(&cache, &path, parse).unwrap();
        release_tx.send(()).unwrap();
        assert_eq!(old.join().unwrap().unwrap().messages.len(), 1);
        let hit = load_with(&cache, &path, |_| panic!("newer fill must remain cached")).unwrap();
        assert_eq!(hit.messages, new.messages);
        assert_eq!(hit.messages.len(), 2);
    });
}

#[test]
fn unknown_revisions_always_load_and_evict_old_entries() {
    let cache = HistoryCache::new();
    cache
        .load("session", || Some(1), || Ok(history("old")))
        .unwrap();
    for text in ["first", "second"] {
        let loaded = cache
            .load("session", || None, || Ok(history(text)))
            .unwrap();
        assert_eq!(loaded.messages, history(text).messages);
    }
    let loaded = cache
        .load("session", || Some(1), || Ok(history("fresh")))
        .unwrap();
    assert_eq!(loaded.messages, history("fresh").messages);
}

#[test]
fn a_hit_protects_an_entry_from_lru_eviction() {
    let cache = HistoryCache::new();
    for index in 0..LIMIT {
        cache
            .load(index, || Some(1), || Ok(history("cached")))
            .unwrap();
    }
    cache
        .load(0, || Some(1), || panic!("expected cache hit"))
        .unwrap();
    cache
        .load(LIMIT, || Some(1), || Ok(history("new")))
        .unwrap();
    cache
        .load(
            0,
            || Some(1),
            || panic!("recently used entry should survive"),
        )
        .unwrap();
    let second = cache
        .load(1, || Some(1), || Ok(history("reloaded")))
        .unwrap();
    assert_eq!(second.messages, history("reloaded").messages);
    assert_eq!(cache.entries.lock().unwrap().len(), LIMIT);
}
