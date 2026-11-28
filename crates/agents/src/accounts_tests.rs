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

#[test]
fn connections_follow_credential_stores_across_projects() {
    let mut state = AccountsSnapshot::default();
    let initial = observation("me@example.com", None, None, 100);
    state.apply(initial.clone());
    state.bind(
        "run".into(),
        &initial.source,
        100,
        AccountBindingEvidence::ObservedAtStartup,
    );
    let connection_id = state.connections[0].id.clone();
    let account_id = state.connections[0].account_id.clone();
    let mut next = initial.clone();
    next.source.context = "/other-project".into();
    next.observed_at = 200;
    state.apply(next.clone());
    assert_eq!(state.connections.len(), 1);
    assert_eq!(state.connections[0].id, connection_id);
    assert_eq!(state.connections[0].account_id, account_id);
    assert_eq!(state.bindings[0].project, "/project");

    next.source.credential_source = "native:other-home".into();
    state.apply(next);
    assert_eq!(state.connections.len(), 2);
    assert_ne!(state.connections[1].account_id, account_id);

    let mut switched = observation("new@example.com", None, None, 300);
    switched.source.context = "/other-project".into();
    switched.limits.clear();
    state.apply(switched);
    assert_eq!(state.connections.len(), 2);
    assert_eq!(state.connections[0].id, connection_id);
    assert_ne!(state.connections[0].account_id, account_id);
    assert_eq!(state.bindings[0].account_id, account_id);
    assert!(state.connections[0].limits.is_empty());
}

#[test]
fn saved_project_duplicates_merge_without_rewriting_run_identity() {
    let mut old = AccountsSnapshot::default();
    let first = observation("old@example.com", None, None, 100);
    old.apply(first.clone());
    old.bind(
        "first".into(),
        &first.source,
        100,
        AccountBindingEvidence::ObservedAtStartup,
    );
    let old_account = old.bindings[0].account_id.clone();
    let mut second = observation("new@example.com", None, None, 200);
    second.source.context = "/other-project".into();
    let mut other = AccountsSnapshot::default();
    other.apply(second.clone());
    other.bind(
        "second".into(),
        &second.source,
        200,
        AccountBindingEvidence::ObservedAtStartup,
    );
    let new_account = other.bindings[0].account_id.clone();
    old.accounts.extend(other.accounts);
    old.connections.extend(other.connections);
    old.bindings.extend(other.bindings);
    let mut json = serde_json::to_value(old).unwrap();
    for binding in json["bindings"].as_array_mut().unwrap() {
        binding.as_object_mut().unwrap().remove("project");
    }
    let mut loaded: AccountsSnapshot = serde_json::from_value(json).unwrap();
    loaded.normalize_connections();
    assert_eq!(loaded.connections.len(), 1);
    assert_eq!(loaded.connections[0].account_id, new_account);
    assert_eq!(loaded.bindings[0].account_id, old_account);
    assert_eq!(loaded.bindings[1].account_id, new_account);
    assert_eq!(loaded.bindings[0].project, "/project");
    assert_eq!(loaded.bindings[1].project, "/other-project");
    assert!(
        loaded
            .bindings
            .iter()
            .all(|b| b.connection_id == loaded.connections[0].id)
    );
}

#[test]
fn legacy_environment_connections_keep_history_without_remaining_available() {
    let mut state = AccountsSnapshot::default();
    let mut old = observation("me@example.com", Some("principal"), None, 100);
    old.source.credential_source = "environment:ANTHROPIC_API_KEY".into();
    state.apply(old.clone());
    state.bind(
        "run".into(),
        &old.source,
        100,
        AccountBindingEvidence::ObservedAtStartup,
    );
    let old_account = state.bindings[0].account_id.clone();
    state.normalize_connections();
    assert!(!state.connections[0].available);
    assert_eq!(state.bindings[0].account_id, old_account);
    old.source.credential_source = "environment:[\"ANTHROPIC_API_KEY\",null]".into();
    state.apply(old);
    state.normalize_connections();
    assert_eq!(state.connections.len(), 2);
    assert!(state.connections[1].available);
    assert_eq!(state.connections[1].account_id, old_account);
    assert_eq!(state.accounts.len(), 1);
}

#[test]
fn native_limits_follow_the_observed_identity_without_changing_auth_or_other_accounts() {
    let mut state = AccountsSnapshot::default();
    let current = observation("current@example.com", None, None, 100);
    state.apply(current.clone());
    let mut worker = current.clone();
    worker.source.incarnation = Some("worker".into());
    state.apply(worker.clone());
    let mut other = worker.clone();
    other.source.incarnation = Some("other-worker".into());
    other.identity.as_mut().unwrap().email = Some("other@example.com".into());
    state.apply(other);
    let mut latest = worker.limits[0].clone();
    latest.remaining_percent = Some(5.);
    latest.observed_at = 200;
    state.observe_limits(&worker.source, vec![latest.clone()]);
    assert_eq!(state.connections[0].limits[0], latest);
    assert_eq!(state.connections[1].limits[0], latest);
    assert_eq!(state.connections[2].limits[0].observed_at, 100);
    state.observe_limits(&worker.source, worker.limits.clone());
    assert_eq!(state.connections[1].limits[0], latest);
    state.connections[1].auth = AccountAuthState::Unknown;
    latest.observed_at = 300;
    state.observe_limits(&worker.source, vec![latest]);
    assert_eq!(state.connections[0].limits[0].observed_at, 200);
    assert_eq!(state.connections[1].auth, AccountAuthState::Unknown);
}

#[test]
fn resumed_process_keeps_known_limits_only_after_matching_the_store_identity() {
    let mut state = AccountsSnapshot::default();
    let mut saved = observation("me@example.com", None, None, 100);
    saved.identity.as_mut().unwrap().plan = Some("pro".into());
    state.apply(saved.clone());
    state.connections[0].auth = AccountAuthState::Unknown;
    let mut resumed = saved;
    resumed.source.incarnation = Some("resumed-process".into());
    resumed.identity.as_mut().unwrap().plan = None;
    resumed.complete_limits = false;
    resumed.limits.clear();
    resumed.observed_at = 200;
    state.apply(resumed.clone());
    assert_eq!(state.connections[1].limits, state.connections[0].limits);
    assert_eq!(state.connections[1].limits[0].observed_at, 100);

    resumed.source.incarnation = Some("other-account".into());
    resumed.identity.as_mut().unwrap().email = Some("other@example.com".into());
    state.apply(resumed.clone());
    assert!(state.connections[2].limits.is_empty());
    resumed.source.credential_source = "native:other-store".into();
    resumed.identity.as_mut().unwrap().email = Some("me@example.com".into());
    state.apply(resumed);
    assert!(state.connections[3].limits.is_empty());
}
