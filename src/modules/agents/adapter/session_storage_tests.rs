use super::*;
use crate::agents::Backend;

const PI_HEADER: &str = "{\"type\":\"session\",\"id\":\"root\",\"cwd\":\"/project\"}\n";

fn summary(harness: Backend, path: PathBuf, id: &str) -> SessionSummary {
    SessionSummary::from_cached_for_harness(
        id.into(),
        harness,
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
fn unsupported_moves_never_touch_files_or_create_destination() {
    let temp = tempfile::tempdir().expect("test fixture");
    let source = temp.path().join("session.jsonl");
    let contents = r#"{"type":"session","id":"root","cwd":"/project"}"#;
    std::fs::write(&source, contents).expect("test fixture");
    let destination = temp.path().join("destination");
    for harness in [Backend::Cursor, Backend::Claude, Backend::Antigravity] {
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
        summary(Backend::Pi, root.clone(), "root"),
        summary(Backend::Codex, temp.path().join("codex-cli/child"), "child"),
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
    for harness in [Backend::Codex, Backend::Cursor, Backend::OpenCode] {
        let targets = [
            summary(Backend::Pi, root.clone(), "root").target(),
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
    assert!(load_session_history(Backend::Pi, &path).is_ok());
    assert!(delete_session_family(&[summary(Backend::Pi, path.clone(), "root").target()]).is_ok());
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
    assert!(load_session_history(Backend::Pi, &path).is_ok());
    for harness in [Backend::Codex, Backend::Cursor, Backend::OpenCode] {
        assert!(load_session_history(harness, &path).is_err(), "{harness}");
    }
}

#[test]
fn external_identity_must_match_both_harness_and_id() {
    let path = PathBuf::from("/locators/codex-cli/thread");
    assert!(
        validate_session_target(&summary(Backend::Codex, path.clone(), "thread").target()).is_ok()
    );
    assert!(
        validate_session_target(&summary(Backend::Codex, path.clone(), "other").target()).is_err()
    );
    assert!(validate_session_target(&summary(Backend::Cursor, path, "thread").target()).is_err());
}
