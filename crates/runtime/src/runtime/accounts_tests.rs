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

#[test]
fn accounts_discard_late_refresh_and_schedule_a_replacement() {
    let (mut service, _, tx, _) = service();
    let initial = observation("old@example.com", 100);
    let scope = source_scope(&initial.source);
    service.pending.insert(scope.clone(), service.generation);
    service
        .sender
        .send((scope.clone(), (service.generation, Ok(vec![initial]))))
        .unwrap();
    service.invalidate();
    service.next_refresh = Instant::now() + Duration::from_secs(300);
    service.tick(&AgentLaunchConfig::default(), Path::new("/project"), &tx);
    assert!(service.snapshot.connections.is_empty());
    assert!(!service.pending.contains_key(&scope));
    assert!(service.next_refresh <= Instant::now());
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
fn accounts_login_invalidation_reaches_profiles_sharing_the_store() {
    let (mut service, events, tx, _) = service();
    let initial = observation("old@example.com", 100);
    service.snapshot.apply(initial.clone());
    let mut profile = initial;
    profile.source.profile_id = Some("other".into());
    service.snapshot.apply(profile);
    events
        .send(AccountEvent::LoginChanged {
            backend: Backend::Codex,
            profile_id: None,
        })
        .unwrap();
    service.tick(&AgentLaunchConfig::default(), Path::new("/project"), &tx);
    assert!(
        service
            .snapshot
            .connections
            .iter()
            .all(|c| c.auth == AccountAuthState::Unknown)
    );
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
