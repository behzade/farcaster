use super::*;
use agents::accounts::{
    AccountAuthState, AccountEvent, AccountsSnapshot, account_now, subscribe_accounts,
};
use std::path::Path;

type ReadResult = (
    u64,
    Result<Vec<agents::accounts::AccountObservation>, String>,
);

pub(super) struct AccountService {
    snapshot: AccountsSnapshot,
    state: Option<farcaster_storage::SharedStateStore>,
    events: mpsc::Receiver<AccountEvent>,
    reads: mpsc::Receiver<(String, ReadResult)>,
    sender: mpsc::Sender<(String, ReadResult)>,
    generation: u64,
    pending: HashMap<String, u64>,
    next_refresh: Instant,
    last_refresh: Option<Instant>,
    dirty: bool,
    enabled: bool,
    active: bool,
    project: Option<std::path::PathBuf>,
    profiles: Vec<agents::HarnessProfile>,
    proxy: Option<String>,
}

impl AccountService {
    pub(super) fn new(
        mut state: Option<farcaster_storage::SharedStateStore>,
        enabled: bool,
    ) -> Self {
        let loaded = state
            .as_ref()
            .map(|s| s.with(|s| s.load_accounts()))
            .transpose();
        let mut snapshot = match loaded {
            Ok(snapshot) => snapshot.unwrap_or_default(),
            Err(error) => {
                zlog::warn!("Account metadata could not be loaded; persistence disabled: {error}");
                state = None;
                AccountsSnapshot::default()
            }
        };
        for connection in &mut snapshot.connections {
            if connection.auth != AccountAuthState::SignedOut {
                connection.auth = AccountAuthState::Unknown;
            }
        }
        for binding in &mut snapshot.bindings {
            if binding.ended_at.is_none() {
                binding.ended_at = Some(account_now());
            }
        }
        let active = snapshot
            .connections
            .iter()
            .any(|c| c.available && c.source.incarnation.is_none());
        let (sender, reads) = mpsc::channel();
        Self {
            snapshot,
            state,
            events: subscribe_accounts(thread::current()),
            reads,
            sender,
            generation: 0,
            pending: HashMap::new(),
            next_refresh: Instant::now(),
            last_refresh: None,
            dirty: true,
            enabled,
            active,
            project: None,
            profiles: Vec::new(),
            proxy: None,
        }
    }

    pub(super) fn refresh(&mut self) {
        self.active = true;
        self.invalidate();
    }

    pub(super) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.next_refresh = Instant::now();
    }

    fn invalidate_connections(
        &mut self,
        matches: impl Fn(&agents::accounts::AccountConnection) -> bool,
    ) {
        let mut ids = HashSet::new();
        for connection in &mut self.snapshot.connections {
            if matches(connection) {
                connection.auth = AccountAuthState::Unknown;
                ids.insert(connection.id.clone());
            }
        }
        for binding in &mut self.snapshot.bindings {
            if binding.ended_at.is_none() && ids.contains(&binding.connection_id) {
                binding.ended_at = Some(account_now());
            }
        }
    }

    pub(super) fn tick(
        &mut self,
        command: &AgentLaunchConfig,
        project: &Path,
        tx: &supervisor::UiEventSender,
    ) {
        if let Ok(profiles) = command.profiles.list()
            && (profiles != self.profiles || command.app_proxy != self.proxy)
        {
            self.profiles = profiles;
            self.proxy = command.app_proxy.clone();
            self.invalidate();
            for connection in &mut self.snapshot.connections {
                if let Some(profile) = &connection.source.profile_id
                    && !self.profiles.iter().any(|p| &p.id == profile)
                {
                    connection.available = false;
                }
            }
            self.dirty = true;
        }
        while let Ok(event) = self.events.try_recv() {
            match event {
                AccountEvent::Observed {
                    observation,
                    run_id,
                    evidence,
                } => {
                    let source = observation.source.clone();
                    let at = observation.observed_at;
                    self.snapshot.apply(observation);
                    if let Some(run_id) = run_id {
                        self.snapshot.bind(run_id, &source, at, evidence);
                    }
                    self.dirty = true;
                }
                AccountEvent::AuthInvalidated { source } => {
                    self.invalidate_connections(|connection| {
                        connection.source.backend == source.backend
                            && connection.source.credential_source == source.credential_source
                    });
                    self.invalidate();
                    self.dirty = true;
                }
                AccountEvent::Disconnected { source } => {
                    if let Some(connection) = self
                        .snapshot
                        .connections
                        .iter_mut()
                        .find(|c| c.source == source)
                    {
                        connection.available = false;
                        connection.auth = AccountAuthState::Unknown;
                        for binding in &mut self.snapshot.bindings {
                            if binding.connection_id == connection.id && binding.ended_at.is_none()
                            {
                                binding.ended_at = Some(account_now());
                            }
                        }
                    }
                    self.dirty = true;
                }
                AccountEvent::LimitsInvalidated { .. } => {
                    self.next_refresh = self.next_refresh.min(
                        self.last_refresh
                            .map_or_else(Instant::now, |at| at + Duration::from_secs(60)),
                    );
                }
                AccountEvent::LoginChanged {
                    backend,
                    profile_id,
                } => {
                    let stores: HashSet<_> = self
                        .snapshot
                        .connections
                        .iter()
                        .filter(|c| {
                            c.source.backend == backend && c.source.profile_id == profile_id
                        })
                        .map(|c| c.source.credential_source.clone())
                        .collect();
                    self.invalidate_connections(|c| {
                        c.source.backend == backend
                            && (c.source.profile_id == profile_id
                                || stores.contains(&c.source.credential_source))
                    });
                    self.invalidate();
                    self.dirty = true;
                }
            }
        }
        while let Ok((scope, (generation, result))) = self.reads.try_recv() {
            self.pending.remove(&scope);
            if generation != self.generation {
                self.next_refresh = Instant::now();
                continue;
            }
            match result {
                Ok(observations) => {
                    let keys: HashSet<_> = observations.iter().map(|o| o.source.key()).collect();
                    // A complete discovery can remove a credential without removing its history.
                    for connection in &mut self.snapshot.connections {
                        if connection.source.incarnation.is_none()
                            && source_scope(&connection.source) == scope
                            && !keys.contains(&connection.source.key())
                        {
                            connection.auth = AccountAuthState::SignedOut;
                            connection.limits.clear();
                        }
                    }
                    for observation in observations {
                        self.snapshot.apply(observation);
                    }
                }
                Err(_) => {
                    for connection in &mut self.snapshot.connections {
                        if source_scope(&connection.source) == scope {
                            connection.last_attempt_at = account_now();
                            connection.refresh_failed = true;
                        }
                    }
                }
            }
            self.dirty = true;
        }
        if self.project.as_deref() != Some(project) {
            self.project = Some(project.to_owned());
            self.invalidate();
        }
        if self.enabled && self.active && Instant::now() >= self.next_refresh {
            self.last_refresh = Some(Instant::now());
            self.next_refresh = Instant::now() + Duration::from_secs(300);
            let profiles = self.profiles.clone();
            let targets = Backend::ALL
                .iter()
                .copied()
                .map(|b| (b, None))
                .chain(profiles.into_iter().map(|p| (p.backend, Some(p.id))));
            for (backend, profile_id) in targets {
                if !agents::account_capabilities(backend).discover {
                    continue;
                }
                let scope =
                    serde_json::to_string(&(backend, &profile_id, project.to_string_lossy()))
                        .expect("account scope serializes");
                if self.pending.contains_key(&scope) {
                    continue;
                }
                let mut config = command.clone();
                config.profile_id = profile_id;
                let project = project.to_owned();
                let generation = self.generation;
                let tx = self.sender.clone();
                let wake = thread::current();
                self.pending.insert(scope.clone(), generation);
                thread::spawn(move || {
                    let result = agents::read_accounts(&config, backend, &project);
                    let _ = tx.send((scope, (generation, result)));
                    wake.unpark();
                });
            }
        }
        if self.dirty {
            // Persist the exact projection published to every account surface.
            if let Some(state) = &self.state
                && let Err(error) = state.with(|s| s.save_accounts(&self.snapshot))
            {
                zlog::warn!("Account metadata could not be saved: {error}");
            }
            let _ = tx.send(RuntimeEvent::AccountsChanged(Arc::new(
                self.snapshot.clone(),
            )));
            self.dirty = false;
        }
    }
}

fn source_scope(source: &agents::accounts::AccountSource) -> String {
    serde_json::to_string(&(source.backend, &source.profile_id, &source.context))
        .expect("account scope serializes")
}

#[cfg(test)]
#[path = "accounts_tests.rs"]
mod tests;
