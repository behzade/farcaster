use super::*;

fn observation(
    email: &str,
    principal: Option<&str>,
    profile: Option<&str>,
    at: i64,
) -> AccountObservation {
    AccountObservation {
        source: AccountSource {
            backend: Backend::Codex,
            profile_id: profile.map(str::to_owned),
            context: "/project".into(),
            provider: "openai".into(),
            credential_source: "native:home".into(),
            incarnation: None,
        },
        identity: Some(AccountIdentity {
            authority: "openai".into(),
            email: Some(email.into()),
            principal_id: principal.map(str::to_owned),
            ..Default::default()
        }),
        auth: AccountAuthState::SignedIn,
        limits: vec![AccountLimit {
            key: "weekly".into(),
            label: "All models".into(),
            pool_id: None,
            duration_seconds: Some(604800),
            remaining_percent: Some(40.),
            value: None,
            resets_at: Some(2000),
            observed_at: at,
        }],
        complete_limits: true,
        limits_failed: false,
        observed_at: at,
        capabilities: AccountCapabilities::default(),
    }
}

#[test]
fn login_change_preserves_history_and_rejects_older_observations() {
    let mut state = AccountsSnapshot::default();
    let a = observation("a@example.com", Some("a"), None, 100);
    state.apply(a.clone());
    state.bind(
        "worker".into(),
        &a.source,
        100,
        AccountBindingEvidence::ObservedAtStartup,
    );
    let old_id = state.connections[0].account_id.clone();
    let mut b = observation("b@example.com", Some("b"), None, 200);
    b.limits.clear();
    state.apply(b);
    state.apply(a);
    assert_ne!(state.connections[0].account_id, old_id);
    assert_eq!(state.bindings[0].account_id, old_id);
    assert!(state.connections[0].limits.is_empty());
    assert_eq!(state.accounts.len(), 2);
}

#[test]
fn email_is_visible_but_only_verified_principals_merge_connections() {
    let mut state = AccountsSnapshot::default();
    state.apply(observation("same@example.com", None, None, 100));
    state.apply(observation("same@example.com", None, Some("profile"), 100));
    assert_eq!(state.accounts.len(), 2);
    let mut verified = AccountsSnapshot::default();
    verified.apply(observation("old@example.com", Some("principal"), None, 100));
    verified.apply(observation(
        "new@example.com",
        Some("principal"),
        Some("profile"),
        200,
    ));
    assert_eq!(verified.accounts.len(), 1);
    assert_eq!(
        verified.accounts[0].identity.display_name(),
        "new@example.com"
    );
    assert_eq!(
        verified.connections[0].account_id,
        verified.connections[1].account_id
    );
}

#[test]
fn partial_windows_preserve_other_windows_without_renewing_their_age() {
    let mut state = AccountsSnapshot::default();
    state.apply(observation("a@example.com", Some("a"), None, 100));
    let mut partial = observation("a@example.com", Some("a"), None, 200);
    partial.complete_limits = false;
    partial.limits[0].key = "short".into();
    state.apply(partial);
    assert_eq!(state.connections[0].limits.len(), 2);
    assert_eq!(state.connections[0].limits[0].observed_at, 100);
    assert!(!state.connections[0].limits[0].is_fresh(2001));
    let mut logout = observation("", None, None, 300);
    logout.identity = None;
    logout.auth = AccountAuthState::SignedOut;
    logout.limits.clear();
    state.apply(logout);
    assert!(state.connections[0].limits.is_empty());
    assert_eq!(state.accounts.len(), 1);
}

#[test]
fn complete_snapshot_keeps_known_metadata_for_sparse_matching_windows() {
    let mut state = AccountsSnapshot::default();
    state.apply(observation("a@example.com", Some("a"), None, 100));
    let mut fresh = observation("a@example.com", Some("a"), None, 200);
    fresh.limits[0].duration_seconds = None;
    fresh.limits[0].resets_at = None;
    state.apply(fresh);
    assert_eq!(
        state.connections[0].limits[0].duration_seconds,
        Some(604800)
    );
    assert_eq!(state.connections[0].limits[0].resets_at, Some(2000));
    assert_eq!(state.connections[0].limits[0].observed_at, 200);
}
