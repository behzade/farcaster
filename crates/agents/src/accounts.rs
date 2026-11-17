//! Account metadata and observations. Credentials remain owned by the adapter.
use std::{
    sync::{Mutex, OnceLock, mpsc},
    thread::Thread,
};

use crate::Backend;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AccountIdentity {
    pub authority: String,
    pub principal_id: Option<String>,
    pub workspace_id: Option<String>,
    pub email: Option<String>,
    pub label: Option<String>,
    pub workspace: Option<String>,
    pub plan: Option<String>,
}

impl AccountIdentity {
    pub fn display_name(&self) -> &str {
        self.email
            .as_deref()
            .or(self.label.as_deref())
            .unwrap_or(&self.authority)
    }

    fn same_principal(&self, other: &Self) -> bool {
        self.principal_id.is_some()
            && self.authority == other.authority
            && self.principal_id == other.principal_id
            && self.workspace_id == other.workspace_id
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountAuthState {
    #[default]
    Unknown,
    Configured,
    SignedIn,
    SignedOut,
    NeedsAuth,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountCapabilities {
    pub discover: bool,
    pub identity: bool,
    pub limits: bool,
    pub login: bool,
    pub logout: bool,
    pub external_changes: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountSource {
    pub backend: Backend,
    pub profile_id: Option<String>,
    /// Project where the adapter resolved this source; provenance, not identity.
    pub context: String,
    pub provider: String,
    pub credential_source: String,
    /// Distinguishes a live backend process from the next-launch connection.
    #[serde(default)]
    pub incarnation: Option<String>,
}

impl AccountSource {
    pub(crate) fn for_run(mut self) -> Self {
        self.incarnation = Some(uuid::Uuid::new_v4().to_string());
        self
    }

    pub fn key(&self) -> String {
        serde_json::to_string(&(
            self.backend,
            &self.profile_id,
            &self.credential_source,
            &self.provider,
            &self.incarnation,
        ))
        .expect("account source serializes")
    }

    pub fn same_store(&self, other: &Self) -> bool {
        self.backend == other.backend
            && self.profile_id == other.profile_id
            && self.credential_source == other.credential_source
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AccountQuotaValue {
    Balance {
        remaining: f64,
        unit: String,
    },
    Usage {
        used: f64,
        limit: Option<f64>,
        unit: String,
    },
    Unlimited,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountLimit {
    pub key: String,
    pub label: String,
    pub pool_id: Option<String>,
    pub duration_seconds: Option<u64>,
    pub remaining_percent: Option<f64>,
    #[serde(default)]
    pub value: Option<AccountQuotaValue>,
    pub resets_at: Option<i64>,
    pub observed_at: i64,
}

impl AccountLimit {
    pub fn is_fresh(&self, now: i64) -> bool {
        now >= self.observed_at
            && now - self.observed_at <= 600
            && self.resets_at.is_none_or(|reset| reset > now)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountObservation {
    pub source: AccountSource,
    pub identity: Option<AccountIdentity>,
    pub auth: AccountAuthState,
    pub limits: Vec<AccountLimit>,
    /// False means that omitted windows are unchanged, not removed.
    pub complete_limits: bool,
    pub limits_failed: bool,
    pub observed_at: i64,
    pub capabilities: AccountCapabilities,
}

/// Complete discovery of one credential store, including when it has no accounts.
pub struct AccountDiscovery {
    pub source: AccountSource,
    pub observations: Vec<AccountObservation>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub identity: AccountIdentity,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountConnection {
    pub available: bool,
    pub id: String,
    pub source: AccountSource,
    pub account_id: Option<String>,
    pub auth: AccountAuthState,
    pub capabilities: AccountCapabilities,
    pub limits: Vec<AccountLimit>,
    pub observed_at: i64,
    pub last_attempt_at: i64,
    pub refresh_failed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountBindingEvidence {
    ObservedAtStartup,
    Confirmed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunAccountBinding {
    pub run_id: String,
    pub connection_id: String,
    pub account_id: Option<String>,
    pub observed_at: i64,
    pub ended_at: Option<i64>,
    pub evidence: AccountBindingEvidence,
    #[serde(default)]
    pub project: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AccountsSnapshot {
    pub accounts: Vec<Account>,
    pub connections: Vec<AccountConnection>,
    pub bindings: Vec<RunAccountBinding>,
}

impl AccountsSnapshot {
    /// Fold older project-scoped records into their credential-source connection.
    pub fn normalize_connections(&mut self) {
        for binding in &mut self.bindings {
            if binding.project.is_empty()
                && let Some(connection) = self
                    .connections
                    .iter()
                    .find(|c| c.id == binding.connection_id)
            {
                binding.project = connection.source.context.clone();
            }
        }
        let mut connections: Vec<AccountConnection> = Vec::new();
        for mut connection in std::mem::take(&mut self.connections) {
            if let Some(current) = connections
                .iter_mut()
                .find(|c| c.source.key() == connection.source.key())
            {
                for binding in &mut self.bindings {
                    if binding.connection_id == connection.id {
                        binding.connection_id = current.id.clone();
                    }
                }
                if (connection.observed_at, connection.last_attempt_at)
                    >= (current.observed_at, current.last_attempt_at)
                {
                    connection.id = current.id.clone();
                    *current = connection;
                }
            } else {
                connections.push(connection);
            }
        }
        self.connections = connections;
    }

    pub fn account(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }

    pub fn apply(&mut self, observation: AccountObservation) {
        let key = observation.source.key();
        let previous = self.connections.iter().find(|c| c.source.key() == key);
        if previous.is_some_and(|c| c.observed_at > observation.observed_at) {
            return;
        }
        let account_id = observation.identity.as_ref().map(|identity| {
            // Display-only identities stay local to a connection. Never merge by email.
            let existing = self
                .accounts
                .iter()
                .find(|a| a.identity.same_principal(identity))
                .or_else(|| {
                    previous
                        .and_then(|c| c.account_id.as_deref())
                        .and_then(|id| self.account(id))
                        .filter(|a| {
                            a.identity == *identity
                                || (a.identity.authority == identity.authority
                                    && a.identity.principal_id == identity.principal_id
                                    && a.identity.workspace_id == identity.workspace_id
                                    && a.identity.email == identity.email
                                    && a.identity.workspace == identity.workspace)
                        })
                });
            existing
                .map(|a| a.id.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
        });
        if let Some(identity) = observation.identity {
            let id = account_id.clone().expect("identity has account id");
            if let Some(account) = self.accounts.iter_mut().find(|a| a.id == id) {
                account.identity = identity;
            } else {
                self.accounts.push(Account { id, identity });
            }
        }
        let connection = match self.connections.iter_mut().find(|c| c.source.key() == key) {
            Some(connection) => connection,
            None => {
                self.connections.push(AccountConnection {
                    available: true,
                    id: uuid::Uuid::new_v4().to_string(),
                    source: observation.source.clone(),
                    account_id: None,
                    auth: AccountAuthState::Unknown,
                    capabilities: observation.capabilities,
                    limits: Vec::new(),
                    observed_at: 0,
                    last_attempt_at: 0,
                    refresh_failed: false,
                });
                self.connections.last_mut().expect("inserted connection")
            }
        };
        let changed_account = connection.account_id != account_id;
        if changed_account || observation.auth == AccountAuthState::SignedOut {
            connection.limits.clear();
        } else if observation.complete_limits {
            connection
                .limits
                .retain(|old| observation.limits.iter().any(|new| new.key == old.key));
        }
        for limit in observation.limits {
            if limit
                .remaining_percent
                .is_some_and(|p| !p.is_finite() || !(0.0..=100.0).contains(&p))
            {
                continue;
            }
            if let Some(old) = connection
                .limits
                .iter_mut()
                .find(|old| old.key == limit.key)
            {
                if limit.observed_at >= old.observed_at {
                    let mut limit = limit;
                    limit.duration_seconds = limit.duration_seconds.or(old.duration_seconds);
                    limit.resets_at = limit.resets_at.or(old.resets_at);
                    *old = limit;
                }
            } else {
                connection.limits.push(limit);
            }
        }
        connection.available = true;
        connection.account_id = account_id;
        connection.source = observation.source;
        connection.auth = observation.auth;
        connection.capabilities = observation.capabilities;
        connection.observed_at = observation.observed_at;
        connection.last_attempt_at = observation.observed_at;
        connection.refresh_failed = observation.limits_failed;
    }

    pub fn bind(
        &mut self,
        run_id: String,
        source: &AccountSource,
        at: i64,
        evidence: AccountBindingEvidence,
    ) {
        let Some(connection) = self
            .connections
            .iter()
            .find(|c| c.source.key() == source.key())
        else {
            return;
        };
        let connection_id = connection.id.clone();
        let account_id = connection.account_id.clone();
        if let Some(previous) = self
            .bindings
            .iter_mut()
            .rev()
            .find(|b| b.run_id == run_id && b.ended_at.is_none())
        {
            if previous.account_id == account_id && previous.connection_id == connection_id {
                return;
            }
            previous.ended_at = Some(at);
        }
        self.bindings.push(RunAccountBinding {
            run_id,
            connection_id,
            account_id,
            observed_at: at,
            ended_at: None,
            evidence,
            project: source.context.clone(),
        });
    }
}

/// Adapters publish facts or invalidation signals, independent of the selected session.
#[derive(Clone, Debug)]
pub enum AccountEvent {
    Observed {
        observation: AccountObservation,
        run_id: Option<String>,
        evidence: AccountBindingEvidence,
    },
    AuthInvalidated {
        source: AccountSource,
    },
    LimitsInvalidated {
        source: AccountSource,
    },
    LoginChanged {
        backend: Backend,
        profile_id: Option<String>,
    },
    Disconnected {
        source: AccountSource,
    },
}

type Subscriber = (mpsc::Sender<AccountEvent>, Thread);
fn subscribers() -> &'static Mutex<Vec<Subscriber>> {
    static SUBSCRIBERS: OnceLock<Mutex<Vec<Subscriber>>> = OnceLock::new();
    SUBSCRIBERS.get_or_init(Mutex::default)
}

pub fn subscribe_accounts(wake: Thread) -> mpsc::Receiver<AccountEvent> {
    let (tx, rx) = mpsc::channel();
    subscribers()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((tx, wake));
    rx
}

pub(crate) fn publish_account_event(event: AccountEvent) {
    subscribers()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|(tx, wake)| {
            let keep = tx.send(event.clone()).is_ok();
            if keep {
                wake.unpark();
            }
            keep
        });
}

pub fn account_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
#[path = "accounts_tests.rs"]
mod tests;
