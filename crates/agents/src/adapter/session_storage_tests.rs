use super::*;
use crate::Backend;

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
        farcaster_sessions::UsageSummary::default(),
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
fn custom_profile_actions_require_the_matching_saved_profile() {
    let id = uuid::Uuid::new_v4();
    let custom = summary(
        Backend::Codex,
        PathBuf::from(format!("/locators/profiles/{id}/codex-cli/same")),
        "same",
    );
    let config = crate::AgentLaunchConfig::default();
    assert!(
        delete_session_family_with_config(&config, &[custom.target()])
            .expect_err("profile is missing")
            .contains("unknown harness profile")
    );
    assert!(
        move_session_family_with_config(&config, &[custom], Path::new("/project"))
            .expect_err("profile is missing")
            .contains("unknown harness profile")
    );
}

#[test]
fn pi_identity_does_not_depend_on_its_parent_directory_name() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let directory = temp.path().join("codex-cli");
    std::fs::create_dir(&directory).expect("session directory");
    let path = directory.join("root.jsonl");
    std::fs::write(&path, "{\"type\":\"session\",\"id\":\"root\"}\n").expect("Pi header");
    assert!(load_session_history(Backend::Pi, &path, temp.path()).is_ok());
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
    assert!(load_session_history(Backend::Pi, &path, temp.path()).is_ok());
    for harness in [Backend::Codex, Backend::Cursor, Backend::OpenCode] {
        assert!(
            load_session_history(harness, &path, temp.path()).is_err(),
            "{harness}"
        );
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

#[test]
fn warmed_pi_history_still_requires_matching_backend() {
    let temp = tempfile::tempdir().expect("fixture");
    let path = temp.path().join("session.jsonl");
    std::fs::write(&path, PI_HEADER).expect("Pi history");
    let config = crate::AgentLaunchConfig::default();
    for _ in 0..2 {
        load_session_history_for_profile(&config, Backend::Pi, &path, temp.path())
            .expect("warm Pi history");
    }
    for backend in [
        Backend::Codex,
        Backend::Claude,
        Backend::Cursor,
        Backend::OpenCode,
    ] {
        assert!(
            load_session_history_for_profile(&config, backend, &path, temp.path()).is_err(),
            "cached Pi history must not satisfy {backend}"
        );
    }
}

#[test]
fn warmed_pi_history_still_validates_changed_or_deleted_profile() {
    let temp = tempfile::tempdir().expect("fixture");
    let profile = crate::HarnessProfile {
        id: uuid::Uuid::new_v4().to_string(),
        name: "History fixture".into(),
        backend: Backend::Pi,
        executable: PathBuf::from("pi"),
        data_directory: None,
    };
    let config = crate::AgentLaunchConfig {
        profile_id: Some(profile.id.clone()),
        ..Default::default()
    };
    config
        .profiles
        .replace(vec![profile.clone()])
        .expect("profile");
    let directory = temp.path().join("profiles").join(&profile.id).join("pi");
    std::fs::create_dir_all(&directory).expect("profile directory");
    let path = directory.join("session.jsonl");
    std::fs::write(&path, PI_HEADER).expect("Pi history");
    for _ in 0..2 {
        load_session_history_for_profile(&config, Backend::Pi, &path, temp.path())
            .expect("warm profile history");
    }
    config
        .profiles
        .replace(vec![crate::HarnessProfile {
            backend: Backend::Claude,
            ..profile
        }])
        .expect("changed profile");
    let error = load_session_history_for_profile(&config, Backend::Pi, &path, temp.path())
        .expect_err("changed backend must fail despite cached history");
    assert!(error.contains("uses"), "{error}");
    config.profiles.replace(vec![]).expect("delete profile");
    let error = load_session_history_for_profile(&config, Backend::Pi, &path, temp.path())
        .expect_err("deleted profile must fail despite cached history");
    assert!(error.contains("unknown harness profile"), "{error}");
}

#[test]
fn profile_history_uses_current_claude_data_directory() {
    let temp = tempfile::tempdir().expect("fixture");
    let session_id = uuid::Uuid::new_v4().to_string();
    let mut profile = crate::HarnessProfile {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Custom Claude".into(),
        backend: Backend::Claude,
        executable: PathBuf::from("claude"),
        data_directory: None,
    };
    let config = crate::AgentLaunchConfig {
        profile_id: Some(profile.id.clone()),
        ..Default::default()
    };
    let locator = temp
        .path()
        .join("profiles")
        .join(&profile.id)
        .join("claude")
        .join(&session_id);
    for text in ["first profile directory", "changed profile directory"] {
        let root = temp.path().join(text);
        let project = root.join("projects/fixture");
        std::fs::create_dir_all(&project).expect("Claude project directory");
        let row = serde_json::json!({
            "type": "user", "uuid": "u", "parentUuid": null,
            "message": {"role": "user", "content": text}
        });
        std::fs::write(
            project.join(format!("{session_id}.jsonl")),
            format!("{row}\n"),
        )
        .expect("Claude history");
        profile.data_directory = Some(root);
        config
            .profiles
            .replace(vec![profile.clone()])
            .expect("profile");
        for _ in 0..2 {
            let history =
                load_session_history_for_profile(&config, Backend::Claude, &locator, temp.path())
                    .expect("custom profile history");
            assert_eq!(history.messages.len(), 1);
            assert_eq!(history.messages[0]["content"][0]["text"], text);
        }
    }
}
