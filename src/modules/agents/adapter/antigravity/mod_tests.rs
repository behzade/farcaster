use super::*;

#[test]
fn launch_requires_the_matching_helper_next_to_the_server() {
    let directory = tempfile::tempdir().expect("test operation should succeed");
    let mut command = std::process::Command::new(directory.path().join(PROFILE.command));
    assert!(
        configure(&mut command)
            .expect_err("invalid test input must fail")
            .contains("matching helper")
    );
    let helper = directory.path().join(if cfg!(windows) {
        "localharness_external.exe"
    } else {
        "localharness_external"
    });
    std::fs::write(&helper, "fixture").expect("test operation should succeed");
    configure(&mut command).expect("test operation should succeed");
    assert!(
        command
            .get_envs()
            .any(|(key, value)| key == "ANTIGRAVITY_HARNESS_PATH"
                && value == Some(helper.as_os_str()))
    );
}

#[test]
fn history_uses_the_conversation_working_directory_from_the_stored_meta() {
    let home = tempfile::tempdir().expect("test operation should succeed");
    let conversations = home.path().join("antigravity-acp").join("conversations");
    std::fs::create_dir_all(&conversations).expect("test operation should succeed");
    std::fs::write(
        conversations.join("saved-session.meta"),
        serde_json::json!({"cwd": "/saved/project"}).to_string(),
    )
    .expect("test operation should succeed");
    assert_eq!(
        conversation_project_in(home.path(), "saved-session"),
        Some(std::path::PathBuf::from("/saved/project"))
    );
    assert_eq!(conversation_project_in(home.path(), "missing"), None);
    std::fs::write(conversations.join("broken.meta"), b"not json".to_vec())
        .expect("test operation should succeed");
    assert_eq!(conversation_project_in(home.path(), "broken"), None);
}
