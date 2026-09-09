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
