use super::*;

fn a_stamp(len: u64) -> Stamp {
    Stamp {
        modified: SystemTime::UNIX_EPOCH,
        len,
    }
}

fn remembered(values: &[(&str, u64)]) -> Remembered<String> {
    let mut remembered = Remembered::new(24);
    for (path, len) in values {
        remembered.remember(
            PathBuf::from(path),
            Some(a_stamp(*len)),
            &format!("{path}#{len}"),
        );
    }
    remembered
}

#[test]
fn a_value_is_kept_until_its_source_changes() {
    let path = Path::new("/chats/one.jsonl");
    let mut remembered = remembered(&[("/chats/one.jsonl", 12)]);
    assert_eq!(
        remembered.recall(path, Some(&a_stamp(12))).as_deref(),
        Some("/chats/one.jsonl#12")
    );
    assert!(
        remembered.recall(path, Some(&a_stamp(13))).is_none(),
        "a source that grew must be read again"
    );
}

#[test]
fn a_value_that_cannot_be_stamped_is_never_reused() {
    let mut remembered = Remembered::new(24);
    let path = PathBuf::from("/chats/gone.jsonl");
    remembered.remember(path.clone(), None, &"value".to_owned());
    assert!(remembered.recall(&path, None).is_none());
    assert!(
        remembered.recall(&path, Some(&a_stamp(1))).is_none(),
        "an unstamped value is not reused even when a stamp turns up later"
    );
}

#[test]
fn the_least_recently_used_value_is_dropped_once_the_limit_is_reached() {
    let mut remembered = Remembered::new(2);
    for path in ["/a", "/b", "/c"] {
        remembered.remember(PathBuf::from(path), Some(a_stamp(1)), &path.to_owned());
    }
    assert_eq!(remembered.recall(Path::new("/a"), Some(&a_stamp(1))), None);
    assert_eq!(
        remembered
            .recall(Path::new("/b"), Some(&a_stamp(1)))
            .as_deref(),
        Some("/b")
    );
}

#[test]
fn a_hit_counts_as_a_use() {
    let mut remembered = Remembered::new(2);
    remembered.remember(PathBuf::from("/keep"), Some(a_stamp(1)), &"keep".to_owned());
    for miss in 0..8 {
        remembered.remember(
            PathBuf::from(format!("/other-{miss}")),
            Some(a_stamp(1)),
            &"other".to_owned(),
        );
        assert!(
            remembered
                .recall(Path::new("/keep"), Some(&a_stamp(1)))
                .is_some(),
            "a value read again and again stays remembered"
        );
    }
}

#[test]
fn forgetting_a_key_leaves_the_other_values_alone() {
    let mut remembered = remembered(&[("/a", 1), ("/b", 1)]);
    remembered.forget(Path::new("/a"));
    assert!(
        remembered
            .recall(Path::new("/a"), Some(&a_stamp(1)))
            .is_none()
    );
    assert!(remembered.contains(Path::new("/b"), Some(&a_stamp(1))));
}

#[test]
fn remembering_a_key_again_replaces_its_value() {
    let path = Path::new("/stats/project");
    let mut remembered = Remembered::new(24);
    remembered.remember(path.to_path_buf(), Some(a_stamp(1)), &"first".to_owned());
    remembered.remember(path.to_path_buf(), Some(a_stamp(2)), &"second".to_owned());
    assert_eq!(
        remembered.recall(path, Some(&a_stamp(2))).as_deref(),
        Some("second")
    );
    assert!(
        remembered.recall(path, Some(&a_stamp(1))).is_none(),
        "the replaced value is gone"
    );
}

#[test]
fn stamps_require_an_absolute_regular_file() {
    let directory = tempfile::tempdir().expect("directory");
    assert!(Stamp::of(directory.path()).is_none());
    assert!(Stamp::of(Path::new("Cargo.toml")).is_none());
    assert!(Stamp::of(&directory.path().join("missing")).is_none());
    let path = directory.path().join("session");
    fs::write(&path, "first").expect("file");
    let first = Stamp::of(&path).expect("stamp");
    fs::write(&path, "longer content").expect("change file");
    assert_ne!(Stamp::of(&path), Some(first));
}
