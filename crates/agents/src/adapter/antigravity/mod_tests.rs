use super::*;

#[test]
fn launch_requires_the_matching_helper_next_to_the_server() {
    let directory = tempfile::tempdir().expect("test operation should succeed");
    let mut command = std::process::Command::new(directory.path().join("agy_acp_server.par"));
    assert!(
        configure(&mut command, crate::HarnessAccessMode::Sandboxed)
            .expect_err("invalid test input must fail")
            .contains("matching helper")
    );
    let helper = directory.path().join(if cfg!(windows) {
        "localharness_external.exe"
    } else {
        "localharness_external"
    });
    std::fs::write(&helper, "fixture").expect("test operation should succeed");
    configure(&mut command, crate::HarnessAccessMode::Sandboxed)
        .expect("test operation should succeed");
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

#[test]
fn integrated_cli_uses_the_existing_store_and_requested_access_mode() {
    for (mode, policy) in [
        (crate::HarnessAccessMode::Sandboxed, "--sandbox"),
        (
            crate::HarnessAccessMode::Full,
            "--dangerously-skip-permissions",
        ),
    ] {
        let mut command = std::process::Command::new(PROFILE.command);
        configure(&mut command, mode).expect("configure integrated CLI");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["acp", "--app-data-dir", "antigravity-acp", policy]
        );
        assert!(
            !command
                .get_envs()
                .any(|(key, _)| key == "ANTIGRAVITY_HARNESS_PATH")
        );
    }
}

#[test]
fn only_a_reported_account_tier_creates_an_account_observation() {
    let response = serde_json::json!({"_meta":{"antigravity":{"tier":{"id":"g1-pro-tier","name":"Google AI Pro"}}}});
    let identity = account_identity(&response).expect("reported tier");
    assert_eq!(identity.plan.as_deref(), Some("Google AI Pro"));
    assert!(identity.principal_id.is_none());
    assert!(identity.email.is_none());
    assert!(account_identity(&serde_json::json!({"_meta":{"antigravity":{"tier":{}}}})).is_none());
}

#[test]
fn tier_metadata_retains_the_id_and_only_an_https_upgrade_link() {
    let identity = |uri| {
        account_identity(&serde_json::json!({"_meta":{"antigravity":{"tier":{
            "name":"Pro","id":"pro-1","upgradeUri":uri
        }}}}))
        .expect("tier")
    };
    assert_eq!(
        identity("https://example.test/plans").plan_id.as_deref(),
        Some("pro-1")
    );
    assert_eq!(
        identity("https://example.test/plans")
            .upgrade_url
            .as_deref(),
        Some("https://example.test/plans")
    );
    assert!(identity("file:///tmp/plan").upgrade_url.is_none());
}

#[cfg(unix)]
#[test]
fn account_actions_use_the_negotiated_method_and_same_store() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = tempfile::tempdir().expect("private test store");
    let agy = directory.path().join("agy");
    std::fs::write(&agy,r#"#!/bin/sh
test "$1 $2 $3" = "acp --app-data-dir antigravity-acp" || exit 1
while IFS= read -r line; do
    id=$(printf '%s' "$line" | sed -n 's/.*"id":\([^,}]*\).*/\1/p')
    case "$line" in
        *'"method":"initialize"'*) result='{"protocolVersion":1,"agentCapabilities":{"auth":{"logout":{}}},"authMethods":[{"id":"gemini-api-key","name":"API key"}]}' ;;
        *'"method":"authenticate"'*'"methodId":"gemini-api-key"'* | *'"method":"logout"'*) result='{}' ;;
        *) exit 2 ;;
    esac
    printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$result"
done
"#).expect("fixture");
    std::fs::set_permissions(&agy, std::fs::Permissions::from_mode(0o700)).expect("executable");
    for method in [Some("gemini-api-key"), None] {
        super::auth::request(std::process::Command::new(&agy), method, &false.into())
            .expect("account action");
    }
    assert!(
        super::auth::request(
            std::process::Command::new(agy),
            Some("unavailable"),
            &false.into()
        )
        .is_err()
    );
}
