pub(crate) use farcaster_storage::*;

use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

static STATE: OnceLock<SharedStateStore> = OnceLock::new();

#[derive(Clone)]
pub(crate) struct SharedStateStore(Arc<Mutex<StateStore>>);

impl SharedStateStore {
    pub(crate) fn new(store: StateStore) -> Self {
        Self(Arc::new(Mutex::new(store)))
    }

    pub(crate) fn lock(&self) -> Result<MutexGuard<'_, StateStore>, String> {
        self.0
            .lock()
            .map_err(|_| "State database lock is poisoned".into())
    }

    pub(crate) fn with<T>(
        &self,
        operation: impl FnOnce(&mut StateStore) -> Result<T, String>,
    ) -> Result<T, String> {
        operation(&mut *self.lock()?)
    }

    pub(crate) fn arc(&self) -> Arc<Mutex<StateStore>> {
        self.0.clone()
    }

    #[cfg(test)]
    pub(crate) fn open_at(path: &std::path::Path) -> Result<Self, String> {
        StateStore::open_at(path).map(Self::new)
    }
}

impl From<StateStore> for SharedStateStore {
    fn from(store: StateStore) -> Self {
        Self::new(store)
    }
}

pub(crate) enum StoreGuard<'a> {
    Shared(MutexGuard<'a, StateStore>),
    #[cfg(test)]
    Owned(StateStore),
}

impl Deref for StoreGuard<'_> {
    type Target = StateStore;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Shared(store) => store,
            #[cfg(test)]
            Self::Owned(store) => store,
        }
    }
}

impl DerefMut for StoreGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Shared(store) => store,
            #[cfg(test)]
            Self::Owned(store) => store,
        }
    }
}

pub(crate) fn initialize() -> Result<SharedStateStore, String> {
    let store = SharedStateStore::new(open_fresh()?);
    STATE
        .set(store.clone())
        .map_err(|_| "State database was already initialized".to_owned())?;
    Ok(store)
}

pub(crate) fn shared() -> Result<SharedStateStore, String> {
    if let Some(store) = STATE.get() {
        return Ok(store.clone());
    }
    #[cfg(test)]
    return open_fresh().map(SharedStateStore::new);
    #[cfg(not(test))]
    Err("State database is not initialized".into())
}

pub(crate) fn open() -> Result<StoreGuard<'static>, String> {
    if let Some(store) = STATE.get() {
        return store.lock().map(StoreGuard::Shared);
    }
    // Unit tests construct isolated databases without running application startup.
    #[cfg(test)]
    return open_fresh().map(StoreGuard::Owned);
    #[cfg(not(test))]
    Err("State database is not initialized".into())
}

fn open_fresh() -> Result<StateStore, String> {
    let _startup_timing =
        crate::app::infrastructure::performance::StartupTiming::new("db.open_total");
    let _timing = crate::app::infrastructure::performance::OperationTiming::new(
        crate::app::infrastructure::performance::OperationKind::StateDatabase,
        1,
    );
    let path = state_path()?;
    let mut store = StateStore::open_at(&path)?;
    if let Some(legacy) = legacy_pi_gpui_state_path()
        && legacy != path
        && legacy.is_file()
    {
        store.import_legacy_pi_gpui_state(&legacy)?;
    }
    Ok(store)
}

pub(crate) fn state_path() -> Result<PathBuf, String> {
    crate::app::infrastructure::paths::data_dir().map(|root| root.join("state.sqlite3"))
}

fn legacy_pi_gpui_state_path() -> Option<PathBuf> {
    let root = std::env::var_os("PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".pi/agent")))?;
    let root = if root.is_absolute() {
        root
    } else {
        std::env::current_dir().ok()?.join(root)
    };
    Some(root.join("gui-state.sqlite3"))
}
