use super::*;
use agents::accounts::*;

fn observation(email: &str, at: i64) -> AccountObservation {
    AccountObservation {
        source: AccountSource {
            backend: Backend::Codex,
            profile_id: None,
            context: "/project".into(),
            provider: "openai".into(),
            credential_source: "native:test".into(),
            incarnation: None,
        },
        identity: Some(AccountIdentity {
            authority: "openai".into(),
            email: Some(email.into()),
            ..Default::default()
        }),
        auth: AccountAuthState::SignedIn,
        limits: vec![AccountLimit {
            key: "weekly".into(),
            label: "Weekly".into(),
            pool_id: None,
            duration_seconds: Some(604800),
            remaining_percent: Some(20.),
            value: None,
            resets_at: None,
            observed_at: at,
        }],
        complete_limits: true,
        limits_failed: false,
        observed_at: at,
        capabilities: AccountCapabilities::default(),
    }
}

fn service() -> (
    AccountService,
    mpsc::Sender<AccountEvent>,
    supervisor::UiEventSender,
    mpsc::Receiver<RuntimeEvent>,
) {
    let mut service = AccountService::new(None, false);
    let (events, receiver) = mpsc::channel();
    service.events = receiver;
    service.project = Some(PathBuf::from("/project"));
    let (tx, rx) = mpsc::channel();
    let (wake, _) = async_channel::bounded(1);
    (
        service,
        events,
        supervisor::UiEventSender { events: tx, wake },
        rx,
    )
}

fn discovered(
    service: &AccountService,
    source: AccountSource,
    observations: Vec<AccountObservation>,
) {
    service
        .sender
        .send((
            source_scope(&source),
            (
                service.generation,
                Ok(AccountDiscovery {
                    source,
                    observations,
                }),
            ),
        ))
        .unwrap();
}

#[test]
fn accounts_legacy_environment_records_remain_hidden_without_starting_discovery() {
    let directory = tempfile::tempdir().unwrap();
    let state =
        farcaster_storage::SharedStateStore::open_at(&directory.path().join("state.sqlite3"))
            .unwrap();
    let mut snapshot = AccountsSnapshot::default();
    let mut old = observation("me@example.com", 100);
    old.source.credential_source = "environment:ANTHROPIC_API_KEY".into();
    snapshot.apply(old);
    state.with(|state| state.save_accounts(&snapshot)).unwrap();
    let service = AccountService::new(Some(state), false);
    assert!(!service.refresh_requested);
    assert!(!service.snapshot.connections[0].available);
}

#[test]
fn accounts_discard_late_refresh_and_schedule_a_replacement() {
    let (mut service, _, tx, _) = service();
    let initial = observation("old@example.com", 100);
    let scope = source_scope(&initial.source);
    service.pending.insert(scope.clone());
    discovered(&service, initial.source.clone(), vec![initial]);
    service.invalidate();
    service.tick(&AgentLaunchConfig::default(), Path::new("/project"), &tx);
    assert!(service.snapshot.connections.is_empty());
    assert!(!service.pending.contains(&scope));
    assert!(service.refresh_requested);
}

#[test]
fn accounts_transport_failure_keeps_last_success_and_its_age() {
    let (mut service, _, tx, _) = service();
    let initial = observation("me@example.com", 100);
    let scope = source_scope(&initial.source);
    service.snapshot.apply(initial);
    service
        .sender
        .send((
            scope,
            (service.generation, Err("network unavailable".into())),
        ))
        .unwrap();
    service.tick(&AgentLaunchConfig::default(), Path::new("/project"), &tx);
    let connection = &service.snapshot.connections[0];
    assert!(connection.refresh_failed);
    assert_eq!(connection.observed_at, 100);
    assert_eq!(connection.limits[0].observed_at, 100);
    assert_eq!(connection.auth, AccountAuthState::SignedIn);
}

#[test]
fn accounts_login_check_preserves_facts_and_guards_profiles_sharing_the_store() {
    let (mut service, events, tx, _) = service();
    let initial = observation("old@example.com", 100);
    service.snapshot.apply(initial.clone());
    let mut profile = initial;
    profile.source.profile_id = Some("other".into());
    service.snapshot.apply(profile);
    let known = service.snapshot.clone();
    events
        .send(AccountEvent::LoginChanged {
            backend: Backend::Codex,
            profile_id: None,
        })
        .unwrap();
    service.tick(&AgentLaunchConfig::default(), Path::new("/project"), &tx);
    assert_eq!(service.snapshot, known);
    assert_eq!(service.awaiting_identity.len(), 2);
}

#[test]
fn accounts_run_observation_does_not_replace_the_next_launch_account() {
    let (mut service, events, tx, _) = service();
    service.snapshot.apply(observation("new@example.com", 200));
    let mut running = observation("old@example.com", 300);
    running.source.incarnation = Some("process-1".into());
    let source = running.source.clone();
    events
        .send(AccountEvent::Observed {
            observation: running,
            run_id: Some("run".into()),
            evidence: AccountBindingEvidence::ObservedAtStartup,
        })
        .unwrap();
    service.tick(&AgentLaunchConfig::default(), Path::new("/project"), &tx);
    let current = &service.snapshot.connections[0];
    assert_eq!(
        service
            .snapshot
            .account(current.account_id.as_deref().unwrap())
            .unwrap()
            .identity
            .email
            .as_deref(),
        Some("new@example.com")
    );
    assert_eq!(service.snapshot.bindings.len(), 1);
    events.send(AccountEvent::Disconnected { source }).unwrap();
    service.tick(&AgentLaunchConfig::default(), Path::new("/project"), &tx);
    assert!(service.snapshot.bindings[0].ended_at.is_some());
    assert_eq!(
        service.snapshot.connections[0].auth,
        AccountAuthState::SignedIn
    );
}

#[test]
fn accounts_login_switch_and_logout_replace_shared_store_across_projects() {
    let (mut service, events, tx, _) = service();
    let config = AgentLaunchConfig::default();
    let project = Path::new("/other-project");
    let initial = observation("old@example.com", 100);
    service.snapshot.apply(initial.clone());
    let mut next = initial;
    next.source.context = "/other-project".into();
    next.observed_at = 200;
    service.snapshot.apply(next.clone());
    service.tick(&config, project, &tx);
    events
        .send(AccountEvent::LoginChanged {
            backend: Backend::Codex,
            profile_id: None,
        })
        .unwrap();
    service.tick(&config, project, &tx);
    next.identity.as_mut().unwrap().email = Some("new@example.com".into());
    next.observed_at = 300;
    discovered(&service, next.source.clone(), vec![next.clone()]);
    service.tick(&config, project, &tx);
    assert_eq!(service.snapshot.connections.len(), 1);
    let current = &service.snapshot.connections[0];
    assert_eq!(
        service
            .snapshot
            .account(current.account_id.as_deref().unwrap())
            .unwrap()
            .identity
            .email
            .as_deref(),
        Some("new@example.com")
    );

    discovered(&service, next.source, vec![]);
    service.tick(&config, project, &tx);
    assert_eq!(service.snapshot.connections.len(), 1);
    assert_eq!(
        service.snapshot.connections[0].auth,
        AccountAuthState::SignedOut
    );
    assert!(service.snapshot.connections[0].account_id.is_none());
    assert!(service.snapshot.connections[0].limits.is_empty());
}

#[test]
fn accounts_empty_discovery_preserves_other_credential_stores_and_live_runs() {
    let (mut service, _, tx, _) = service();
    let initial = observation("me@example.com", 100);
    service.snapshot.apply(initial.clone());
    let mut other = initial.clone();
    other.source.credential_source = "native:other-home".into();
    service.snapshot.apply(other);
    let mut running = initial.clone();
    running.source.incarnation = Some("running".into());
    service.snapshot.apply(running);
    let mut source = initial.source;
    source.context = "/other-project".into();
    discovered(&service, source, vec![]);
    service.tick(&AgentLaunchConfig::default(), Path::new("/project"), &tx);
    assert_eq!(
        service.snapshot.connections[0].auth,
        AccountAuthState::SignedOut
    );
    assert!(
        service.snapshot.connections[1..]
            .iter()
            .all(|c| c.auth == AccountAuthState::SignedIn && !c.limits.is_empty())
    );
}

#[test]
fn accounts_native_limits_update_without_starting_background_reads() {
    let (mut service, events, tx, _) = service();
    service.enabled = true;
    let initial = observation("me@example.com", 100);
    service.snapshot.apply(initial.clone());
    service.invalidate();
    service.tick(&AgentLaunchConfig::default(), Path::new("/project"), &tx);
    assert!(service.pending.is_empty());
    assert!(!service.refresh_requested);

    let mut limit = initial.limits[0].clone();
    limit.remaining_percent = Some(10.);
    limit.observed_at = 200;
    events
        .send(AccountEvent::LimitsObserved {
            source: initial.source,
            limits: vec![limit],
        })
        .unwrap();
    for _ in 0..3 {
        service.tick(&AgentLaunchConfig::default(), Path::new("/project"), &tx);
        assert!(service.pending.is_empty());
        assert!(!service.refresh_requested);
    }
    assert_eq!(
        service.snapshot.connections[0].limits[0].remaining_percent,
        Some(10.)
    );
    assert_eq!(service.snapshot.connections[0].limits[0].observed_at, 200);
}

#[test]
fn persisted_account_facts_do_not_expire_and_old_processes_do_not_become_live() {
    let directory = tempfile::tempdir().unwrap();
    let state =
        farcaster_storage::SharedStateStore::open_at(&directory.path().join("state.sqlite3"))
            .unwrap();
    let mut snapshot = AccountsSnapshot::default();
    let initial = observation("me@example.com", 100);
    snapshot.apply(initial.clone());
    let mut old_worker = initial;
    old_worker.source.incarnation = Some("old-process".into());
    snapshot.apply(old_worker);
    let source = snapshot.connections[1].source.clone();
    snapshot.bind(
        "old-run".into(),
        &source,
        100,
        AccountBindingEvidence::ObservedAtStartup,
    );
    state.with(|state| state.save_accounts(&snapshot)).unwrap();
    let service = AccountService::new(Some(state), false);
    assert_eq!(
        service.snapshot.connections[0].auth,
        AccountAuthState::SignedIn
    );
    assert_eq!(service.snapshot.connections[0].limits[0].observed_at, 100);
    assert!(service.snapshot.connections[0].available);
    assert!(service.snapshot.connections[1].available);
    assert!(service.snapshot.bindings[0].ended_at.is_some());
    assert!(!service.refresh_requested);
}

#[test]
fn worker_disconnect_keeps_observed_account_facts_while_ending_run_binding() {
    let (mut service, events, tx, _) = service();
    let mut known = observation("me@example.com", 100);
    known.source.incarnation = Some("idle-worker".into());
    service.snapshot.apply(known.clone());
    service.snapshot.bind(
        "run".into(),
        &known.source,
        100,
        AccountBindingEvidence::ObservedAtStartup,
    );
    events
        .send(AccountEvent::Disconnected {
            source: known.source,
        })
        .unwrap();
    service.tick(&AgentLaunchConfig::default(), Path::new("/project"), &tx);
    let connection = &service.snapshot.connections[0];
    assert!(connection.available);
    assert_eq!(connection.auth, AccountAuthState::SignedIn);
    assert_eq!(connection.limits, known.limits);
    assert!(service.snapshot.bindings[0].ended_at.is_some());
    assert!(!service.refresh_requested);
}

#[test]
fn native_auth_check_changes_no_facts_and_blocks_unattributed_limits_until_identity_arrives() {
    let (mut service, events, tx, _) = service();
    let config = AgentLaunchConfig::default();
    let project = Path::new("/project");
    let mut known = observation("me@example.com", 100);
    known.source.incarnation = Some("process".into());
    service.snapshot.apply(known.clone());
    let facts = service.snapshot.clone();
    events
        .send(AccountEvent::AuthInvalidated {
            source: known.source.clone(),
        })
        .unwrap();
    let mut limit = known.limits[0].clone();
    limit.observed_at = 200;
    limit.remaining_percent = Some(10.);
    events
        .send(AccountEvent::LimitsObserved {
            source: known.source.clone(),
            limits: vec![limit.clone()],
        })
        .unwrap();
    service.tick(&config, project, &tx);
    assert_eq!(service.snapshot, facts);
    assert!(!service.refresh_requested);

    known.observed_at = 200;
    known.complete_limits = false;
    known.limits.clear();
    events
        .send(AccountEvent::Observed {
            observation: known.clone(),
            run_id: None,
            evidence: AccountBindingEvidence::ObservedAtStartup,
        })
        .unwrap();
    service.tick(&config, project, &tx);
    assert_eq!(
        service.snapshot.connections[0].limits,
        facts.connections[0].limits
    );
    assert!(service.awaiting_identity.is_empty());
    events
        .send(AccountEvent::LimitsObserved {
            source: known.source,
            limits: vec![limit.clone()],
        })
        .unwrap();
    service.tick(&config, project, &tx);
    assert_eq!(service.snapshot.connections[0].limits, vec![limit]);
}
