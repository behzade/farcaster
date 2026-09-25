use super::*;

#[test]
fn login_shell_command_quotes_the_executable_path() {
    assert_eq!(
        login_shell_command(Path::new("/tmp/my shell's bin")),
        "'/tmp/my shell'\\''s bin' -l"
    );
}

#[test]
fn login_shell_relaunch_preserves_explicit_launch_configuration() {
    let environment = preserve_launch_environment(
        vec![
            ("PATH".into(), "/login/bin".into()),
            ("FARCASTER_PI_PATH".into(), "/login/pi".into()),
        ],
        |name| (name == "FARCASTER_PI_PATH").then(|| "/nix/store/pi".into()),
    );

    assert!(environment.contains(&("PATH".into(), "/login/bin".into())));
    assert!(environment.contains(&("FARCASTER_PI_PATH".into(), "/nix/store/pi".into())));
    assert!(!environment.contains(&("FARCASTER_PI_PATH".into(), "/login/pi".into())));
}

#[test]
fn login_shell_relaunch_keeps_the_opencode_model_override() {
    let environment = preserve_launch_environment(
        vec![(
            "FARCASTER_OPENCODE_MODEL".into(),
            "login/shell-model".into(),
        )],
        |name| (name == "FARCASTER_OPENCODE_MODEL").then(|| "opencode-go/big-pickle".into()),
    );

    assert!(environment.contains(&(
        "FARCASTER_OPENCODE_MODEL".into(),
        "opencode-go/big-pickle".into()
    )));
    assert!(!environment.contains(&(
        "FARCASTER_OPENCODE_MODEL".into(),
        "login/shell-model".into()
    )));
}
