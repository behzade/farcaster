use super::*;
use serde_json::json;

#[test]
fn saved_login_requires_matching_backend_and_unexpired_key() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("auth.json");
    let mut credentials = json!({"version":1,"backendUrl":"https://api2.cursor.sh/","apiKey":"saved-key","createdAtMs":1,"apiKeyExpiresAtMs":100});
    std::fs::write(&path, credentials.to_string()).expect("credentials");
    assert_eq!(
        saved_key(&path, "https://api2.cursor.sh", 99.0).expect("read"),
        Some("saved-key".into())
    );
    assert_eq!(
        saved_key(&path, "https://api2.cursor.sh", 100.0).expect("read"),
        None
    );
    assert_eq!(
        saved_key(&path, "https://another.invalid", 99.0).expect("read"),
        None
    );
    credentials
        .as_object_mut()
        .expect("object")
        .remove("apiKeyExpiresAtMs");
    std::fs::write(&path, credentials.to_string()).expect("credentials");
    assert_eq!(
        saved_key(&path, "https://api2.cursor.sh", 101.0).expect("read"),
        Some("saved-key".into())
    );
    std::fs::write(&path, "broken").expect("credentials");
    assert_eq!(
        saved_key(&path, "https://api2.cursor.sh", 99.0).expect("read"),
        None
    );
}

#[test]
fn environment_key_wins_and_profiles_keep_separate_saved_logins() {
    let directory = tempfile::tempdir().expect("directory");
    let mut command = Command::new("unused");
    command
        .env("HOME", directory.path())
        .env_remove("CURSOR_API_KEY")
        .env_remove("CURSOR_BACKEND_URL");
    let mut config = AgentLaunchConfig {
        session_locator_root: Some(directory.path().join("locators")),
        ..Default::default()
    };
    let path = credential_path(&config, &command).expect("path");
    assert_eq!(path, directory.path().join(".cursor/sdk/auth.json"));
    std::fs::create_dir_all(path.parent().expect("parent")).expect("directory");
    std::fs::write(&path, json!({"version":1,"backendUrl":"https://api2.cursor.sh","apiKey":"saved-key","createdAtMs":1}).to_string()).expect("credentials");
    assert_eq!(
        api_key(&config, &command).expect("read"),
        Some("saved-key".into())
    );
    command.env("CURSOR_API_KEY", "explicit-key");
    assert_eq!(
        api_key(&config, &command).expect("read"),
        Some("explicit-key".into())
    );
    command.env_remove("CURSOR_API_KEY");
    config.profile_id = Some("other-account".into());
    assert_eq!(
        credential_path(&config, &command).expect("path"),
        directory
            .path()
            .join("locators/profiles/other-account/cursor-sdk-auth/auth.json")
    );
    assert_eq!(api_key(&config, &command).expect("read"), None);
}

#[test]
fn login_protocol_delivers_browser_url_and_native_error() {
    let mut command = Command::new("sh");
    command.args(["-c", "printf '%s\\n' '{\"type\":\"url\",\"url\":\"https://cursor.com/login\"}' '{\"type\":\"error\",\"message\":\"Account rejected\"}'"]);
    let url = std::cell::RefCell::new(None);
    let result = run(
        command,
        &AtomicBool::new(false),
        &|value| *url.borrow_mut() = Some(value),
        Duration::from_secs(3),
    );
    assert_eq!(*url.borrow(), Some("https://cursor.com/login".into()));
    assert_eq!(result, Err("Account rejected".into()));
}

#[test]
fn cancellation_stops_login_and_no_completion_is_reported() {
    let mut command = Command::new("sh");
    command.args(["-c", "printf '%s\\n' '{\"type\":\"url\",\"url\":\"https://cursor.com/login\"}'; read reply; exit 1"]);
    let cancelled = AtomicBool::new(false);
    let result = run(
        command,
        &cancelled,
        &|_| cancelled.store(true, Ordering::Release),
        Duration::from_secs(3),
    );
    assert_eq!(result, Err("Sign-in cancelled".into()));
}

#[test]
fn sign_in_gate_refreshes_credentials_and_rejects_expired_or_broken_logins() {
    let directory = tempfile::tempdir().unwrap();
    let mut command = Command::new("unused");
    command
        .env("HOME", directory.path())
        .env_remove("CURSOR_API_KEY")
        .env_remove("CURSOR_BACKEND_URL");
    let config = AgentLaunchConfig::default();
    assert!(sign_in_required(&config, &command));
    let path = credential_path(&config, &command).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut credential =
        json!({"version":1,"backendUrl":"https://api2.cursor.sh","apiKey":"key","createdAtMs":1});
    std::fs::write(&path, credential.to_string()).unwrap();
    assert!(!sign_in_required(&config, &command));
    credential["apiKeyExpiresAtMs"] = 1.into();
    std::fs::write(&path, credential.to_string()).unwrap();
    assert!(sign_in_required(&config, &command));
    std::fs::write(&path, "invalid").unwrap();
    assert!(sign_in_required(&config, &command));
    command.env("CURSOR_API_KEY", "explicit-key");
    assert!(!sign_in_required(&config, &command));
}
