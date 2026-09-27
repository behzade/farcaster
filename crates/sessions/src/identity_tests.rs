use super::*;

#[test]
fn application_keys_reject_invalid_and_noncanonical_ids() {
    let id = AppSessionId::new(42).unwrap();
    assert_eq!(AppSessionId::from_key(&id.to_key()), Some(id));
    assert_eq!(serde_json::from_str::<AppSessionId>("42").unwrap(), id);
    for value in [0, -1, i64::MIN] {
        assert!(AppSessionId::new(value).is_none());
        assert!(serde_json::from_str::<AppSessionId>(&value.to_string()).is_err());
    }
    for key in [
        "42",
        "app-session:0",
        "app-session:-1",
        "app-session:042",
        "app-session:+42",
    ] {
        assert!(AppSessionId::from_key(key).is_none(), "{key}");
    }
}

#[test]
fn locator_profile_fallback_requires_a_profile_uuid() {
    let profile = "11111111-1111-4111-8111-111111111111";
    assert_eq!(
        profile_id_from_locator(Path::new(&format!(
            "/locators/profiles/{profile}/codex-cli/id"
        ))),
        Some(profile.to_owned())
    );
    for path in [
        "/pi/sessions/id.jsonl",
        "/locators/codex-cli/id",
        "/locators/profiles/a/codex-cli/id",
    ] {
        assert_eq!(profile_id_from_locator(Path::new(path)), None);
    }
}

#[test]
fn routing_keys_do_not_match_across_binding_or_profiles() {
    let locator = SessionKey::Locator {
        harness: Backend::Pi,
        profile_id: Some("profile-a".into()),
        path: "/pi/id.jsonl".into(),
    };
    let built_in = SessionKey::Locator {
        harness: Backend::Pi,
        profile_id: None,
        path: "/pi/id.jsonl".into(),
    };
    assert_ne!(locator, built_in);
    assert_ne!(locator, SessionKey::App(AppSessionId::new(1).unwrap()));
    assert_eq!(
        serde_json::from_str::<SessionKey>(&serde_json::to_string(&locator).unwrap()).unwrap(),
        locator
    );
}
