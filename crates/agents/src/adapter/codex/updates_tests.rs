use super::*;

#[test]
fn reads_codex_cache_and_only_announces_a_new_release_once() {
    let home = tempfile::tempdir().expect("codex home");
    let path = home.path().join("version.json");
    let agent = "farcaster/0.99.0 (Linux) codex_cli_rs/0.99.0";
    assert!(notice(home.path(), agent).is_none());
    fs::write(&path, "{").expect("partial cache");
    assert!(notice(home.path(), agent).is_none());
    fs::write(
        &path,
        r#"{"latest_version":"0.100.0","dismissed_version":"0.100.0"}"#,
    )
    .expect("dismissed release");
    assert!(notice(home.path(), agent).is_none());
    fs::write(&path, r#"{"latest_version":"0.100.0"}"#).expect("available release");
    for current in ["0.100.0", "0.101.0", "0.0.0", "0.99.0-alpha.1", "unknown"] {
        assert!(notice(home.path(), &format!("farcaster/{current}")).is_none());
    }
    let message = notice(home.path(), agent).expect("new release");
    assert!(message.contains("0.100.0") && message.contains("0.99.0"));
    assert!(notice(home.path(), agent).is_none());
}
