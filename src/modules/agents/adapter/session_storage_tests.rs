use super::*;

const PI_HEADER: &str = "{\"type\":\"session\",\"id\":\"root\",\"cwd\":\"/project\"}\n";

fn summary(harness: &str, path: PathBuf, id: &str) -> SessionSummary {
    SessionSummary::from_cached_for_harness(
        id.into(),
        harness.into(),
        path,
        PathBuf::from("/project"),
        String::new(),
        String::new(),
        String::new(),
        None,
        std::time::SystemTime::now(),
        0,
        crate::sessions::UsageSummary::default(),
        true,
        false,
        String::new(),
    )
}

#[test]
fn pi_discovery_failure_keeps_other_harnesses_and_prevents_pruning() {
    let other = summary(
        "codex-cli",
        PathBuf::from("/locators/codex-cli/thread"),
        "thread",
    );
    let discovery = merge_discovery(Err("unreadable Pi root".into()), vec![other.clone()], true);
    assert_eq!(discovery.sessions, vec![other]);
    assert!(!discovery.exhaustive);
}

#[test]
fn unsupported_moves_never_touch_files_or_create_destination() {
    let temp = tempfile::tempdir().expect("test fixture");
    let source = temp.path().join("session.jsonl");
    let contents = r#"{"type":"session","id":"root","cwd":"/project"}"#;
    std::fs::write(&source, contents).expect("test fixture");
    let destination = temp.path().join("destination");
    for harness in ["cursor-cli", "unknown", ""] {
        let session = summary(harness, source.clone(), "root");
        assert!(move_session_family(&[session], &destination).is_err());
        assert_eq!(
            std::fs::read_to_string(&source).expect("test fixture"),
            contents
        );
        assert!(!destination.exists());
    }
}

#[test]
fn mixed_harness_move_fails_before_pi_mutation() {
    let temp = tempfile::tempdir().expect("test fixture");
    let root = temp.path().join("root.jsonl");
    std::fs::write(&root, PI_HEADER).expect("test fixture");
    let family = [
        summary("pi", root.clone(), "root"),
        summary("codex-cli", temp.path().join("codex-cli/child"), "child"),
    ];
    assert!(
        move_session_family(&family, &temp.path().join("destination"))
            .expect_err("mixed family")
            .contains("across harnesses")
    );
    assert_eq!(
        std::fs::read_to_string(root).expect("test fixture"),
        PI_HEADER
    );
    assert!(!temp.path().join("destination").exists());
}

#[test]
fn deletion_validates_all_members_before_touching_any_file() {
    let temp = tempfile::tempdir().expect("test fixture");
    let root = temp.path().join("root.jsonl");
    std::fs::write(&root, PI_HEADER).expect("test fixture");
    for harness in ["codex-cli", "cursor-cli", "opencode2", "unknown", ""] {
        let targets = [
            summary("pi", root.clone(), "root").target(),
            summary(harness, temp.path().join("unrecognized"), "child").target(),
        ];
        assert!(delete_session_family(&targets).is_err());
        assert_eq!(
            std::fs::read_to_string(&root).expect("test fixture"),
            PI_HEADER
        );
    }
}

#[test]
fn pi_identity_does_not_depend_on_its_parent_directory_name() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let directory = temp.path().join("codex-cli");
    std::fs::create_dir(&directory).expect("session directory");
    let path = directory.join("root.jsonl");
    std::fs::write(&path, "{\"type\":\"session\",\"id\":\"root\"}\n").expect("Pi header");
    assert!(load_session_history("pi", &path).is_ok());
    assert!(delete_session_family(&[summary("pi", path.clone(), "root").target()]).is_ok());
    assert!(!path.exists());
}

#[test]
fn history_requires_explicit_matching_harness_without_pi_fallback() {
    let temp = tempfile::tempdir().expect("test fixture");
    let path = temp.path().join("session.jsonl");
    std::fs::write(
        &path,
        "{\"type\":\"session\",\"id\":\"root\",\"cwd\":\"/project\"}\n",
    )
    .expect("test fixture");
    assert!(load_session_history("pi", &path).is_ok());
    for harness in ["codex-cli", "cursor-cli", "opencode2", "unknown", ""] {
        assert!(load_session_history(harness, &path).is_err(), "{harness}");
    }
}

#[test]
fn external_identity_must_match_both_harness_and_id() {
    let path = PathBuf::from("/locators/codex-cli/thread");
    assert!(
        validate_session_target(&summary("codex-cli", path.clone(), "thread").target()).is_ok()
    );
    assert!(
        validate_session_target(&summary("codex-cli", path.clone(), "other").target()).is_err()
    );
    assert!(validate_session_target(&summary("cursor-cli", path, "thread").target()).is_err());
}
