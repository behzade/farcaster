use super::*;
use agents::accounts::{
    AccountAuthState, AccountEvent, AccountsSnapshot, account_now, subscribe_accounts,
};
use std::path::Path;

type ReadResult = (u64, Result<agents::accounts::AccountDiscovery, String>);

pub(super) struct AccountService {
    snapshot: AccountsSnapshot,
    state: Option<farcaster_storage::SharedStateStore>,
    events: mpsc::Receiver<AccountEvent>,
    reads: mpsc::Receiver<(String, ReadResult)>,
    sender: mpsc::Sender<(String, ReadResult)>,
    generation: u64,
    pending: HashSet<String>,
    awaiting_identity: HashSet<String>,
    refresh_requested: bool,
    dirty: bool,
    enabled: bool,
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
        snapshot.normalize_connections();
        for binding in &mut snapshot.bindings {
            if binding.ended_at.is_none() {
                binding.ended_at = Some(account_now());
            }
        }
        let (sender, reads) = mpsc::channel();
        Self {
            snapshot,
            state,
            events: subscribe_accounts(thread::current()),
            reads,
            sender,
            generation: 0,
            pending: HashSet::new(),
            awaiting_identity: HashSet::new(),
            refresh_requested: false,
            dirty: true,
            enabled,
            project: None,
            profiles: Vec::new(),
            proxy: None,
        }
    }

    pub(super) fn refresh(&mut self) {
        self.invalidate();
        self.refresh_requested = true;
    }

    pub(super) fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.refresh_requested |= !self.pending.is_empty();
    }

    fn await_identity(&mut self, matches: impl Fn(&agents::accounts::AccountConnection) -> bool) {
        let mut ids = HashSet::new();
        for connection in &self.snapshot.connections {
            if matches(connection) {
                // Guard identity-free events while the source is being checked.
                // The check does not change any previously observed account facts.
                self.awaiting_identity.insert(connection.id.clone());
                ids.insert(connection.id.clone());
            }
        }
        for binding in &mut self.snapshot.bindings {
            if binding.ended_at.is_none() && ids.contains(&binding.connection_id) {
                binding.ended_at = Some(account_now());
            }
        }
    }

    fn observe(&mut self, observation: agents::accounts::AccountObservation) {
        let key = observation.source.key();
        let at = observation.observed_at;
        self.snapshot.apply(observation);
        if let Some(connection) = self.snapshot.connections.iter().find(|c| {
            c.source.key() == key && c.observed_at == at && c.auth != AccountAuthState::Unknown
        }) {
            self.awaiting_identity.remove(&connection.id);
        }
    }

    pub(super) fn tick(
        &mut self,
        command: &AgentLaunchConfig,
        project: &Path,
        tx: &supervisor::UiEventSender,
    ) {
        if self.project.as_deref() != Some(project) {
            self.project = Some(project.to_owned());
            self.invalidate();
        }
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
                    self.observe(observation);
                    if let Some(run_id) = run_id {
                        self.snapshot.bind(run_id, &source, at, evidence);
                    }
                    self.dirty = true;
                }
                AccountEvent::AuthInvalidated { source } => {
                    self.await_identity(|connection| {
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
                        .iter()
                        .find(|c| c.source == source)
                    {
                        for binding in &mut self.snapshot.bindings {
                            if binding.connection_id == connection.id && binding.ended_at.is_none()
                            {
                                binding.ended_at = Some(account_now());
                            }
                        }
                    }
                    self.dirty = true;
                }
                AccountEvent::LimitsObserved { source, limits } => {
                    if self.snapshot.connections.iter().any(|c| {
                        c.source.key() == source.key() && self.awaiting_identity.contains(&c.id)
                    }) {
                        continue;
                    }
                    self.snapshot.observe_limits(&source, limits);
                    self.dirty = true;
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
                    self.await_identity(|c| {
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
                self.refresh_requested = true;
                continue;
            }
            match result {
                Ok(discovery) => {
                    let keys: HashSet<_> = discovery
                        .observations
                        .iter()
                        .map(|o| o.source.key())
                        .collect();
                    // A complete discovery can remove a credential without removing its history.
                    for connection in &mut self.snapshot.connections {
                        if connection.source.incarnation.is_none()
                            && connection.source.same_store(&discovery.source)
                            && !keys.contains(&connection.source.key())
                        {
                            connection.auth = AccountAuthState::SignedOut;
                            connection.account_id = None;
                            connection.limits.clear();
                            connection.observed_at = account_now();
                            connection.last_attempt_at = connection.observed_at;
                            connection.refresh_failed = false;
                        }
                    }
                    for observation in discovery.observations {
                        self.observe(observation);
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
        if self.enabled && self.refresh_requested {
            self.refresh_requested = false;
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
                let scope = serde_json::to_string(&(backend, &profile_id))
                    .expect("account scope serializes");
                if self.pending.contains(&scope) {
                    continue;
                }
                let mut config = command.clone();
                config.profile_id = profile_id;
                let project = project.to_owned();
                let generation = self.generation;
                let tx = self.sender.clone();
                let wake = thread::current();
                self.pending.insert(scope.clone());
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
    serde_json::to_string(&(source.backend, &source.profile_id)).expect("account scope serializes")
}

#[cfg(test)]
#[path = "accounts_tests.rs"]
mod tests;
