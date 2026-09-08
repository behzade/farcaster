use super::*;

const PROFILE: AcpProfile = AcpProfile {
    backend: "test-acp",
    name: "Test ACP",
    command: "test-acp",
    path_environment: "FARCASTER_TEST_ACP_PATH",
    arguments: &["acp"],
    auth_method: None,
    force_argument: Some("--force"),
};

#[test]
fn full_access_uses_the_profile_escape_hatch() {
    let mut command = std::process::Command::new("agent");
    configure_command(&mut command, &PROFILE, HarnessAccessMode::Full);
    assert_eq!(
        command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        ["--force", "acp"]
    );
}
