//! SQLite adapter for application state and domain persistence ports.

use farcaster_access as access;
use farcaster_agent_protocol::extensions as protocol;
use farcaster_agents as agents;
use farcaster_projects as projects;
use farcaster_repository as repository;
use farcaster_sessions as sessions;

use crate::agents::Backend;
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rusqlite::{
    Connection, ErrorCode, OptionalExtension as _, Transaction, TransactionBehavior, params,
};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    agents::{PromptPresentation, QueuedPrompt},
    projects::Registry,
    protocol::{PromptImage, PromptMode},
    sessions::{DraftSession, SessionSummary, UsageSummary},
};

mod backend;
mod composer;
mod composer_worker;
#[path = "drafts.rs"]
mod draft_storage;
mod identity;
mod images;
mod migrate_legacy;
mod migrate_v12;
mod migrate_v16;
mod migrate_v17;
mod migrate_v18;
mod migrate_v19;
mod migrate_v20;
#[cfg(test)]
mod persistence_tests;
#[path = "projects.rs"]
mod project_storage;
mod prompts;
mod reviews;
mod schema;
#[path = "sessions.rs"]
mod session_storage;
mod settings;
mod traits;
mod transcript;

pub use composer_worker::ComposerPersistenceWorker;
use draft_storage::{remove_draft_row, save_draft};
use identity::{bind_locator, ensure_locator_session, ensure_project, target_for_session};

const SCHEMA_VERSION: i64 = 20;
const DATABASE_BUSY_TIMEOUT: Duration = Duration::from_secs(10);
const LEGACY_PI_GPUI_IMPORT_KEY: &str = "legacy_pi_gpui_state_imported";
const REPOSITORY_BACKENDS: [&str; 3] = ["auto", "git", "jj"];

pub struct StateStore {
    connection: Connection,
    image_directory: PathBuf,
}

#[derive(Clone)]
pub struct SharedStateStore(Arc<Mutex<StateStore>>);

impl SharedStateStore {
    pub fn new(store: StateStore) -> Self {
        Self(Arc::new(Mutex::new(store)))
    }

    pub fn lock(&self) -> Result<MutexGuard<'_, StateStore>, String> {
        self.0
            .lock()
            .map_err(|_| "State database lock is poisoned".into())
    }

    pub fn with<T>(
        &self,
        operation: impl FnOnce(&mut StateStore) -> Result<T, String>,
    ) -> Result<T, String> {
        operation(&mut *self.lock()?)
    }

    pub fn arc(&self) -> Arc<Mutex<StateStore>> {
        self.0.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn open_at(path: &Path) -> Result<Self, String> {
        StateStore::open_at(path).map(Self::new)
    }
}

impl From<StateStore> for SharedStateStore {
    fn from(store: StateStore) -> Self {
        Self::new(store)
    }
}

impl StateStore {
    pub fn with_connection<T>(&mut self, operation: impl FnOnce(&mut Connection) -> T) -> T {
        operation(&mut self.connection)
    }
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct WindowPlacement {
    pub bounds: [f32; 4],
    pub display_uuid: Option<String>,
    pub display_origin: [f32; 2],
    pub state: WindowState,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub enum WindowState {
    Windowed,
    Maximized,
    Fullscreen,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct CachedConfigurationCatalog {
    pub harness: Backend,
    pub project: PathBuf,
    pub catalog: crate::agents::ConfigurationCatalog,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct CachedSessionControlDefaults {
    pub harness: Backend,
    pub model: Option<crate::protocol::Model>,
    pub effort: Option<String>,
    pub access_mode: Option<crate::agents::HarnessAccessMode>,
}

pub type ComposerRecord = sessions::ComposerRecord<ComposerAttachment>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComposerAttachment {
    Image(PromptImage),
    TextFile { path: PathBuf },
}

fn now_ms() -> u64 {
    system_time_ms(SystemTime::now())
}

fn system_time_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn u64_to_i64(value: u64) -> i64 {
    value.try_into().unwrap_or(i64::MAX)
}

fn usize_to_u64(value: usize) -> u64 {
    value.try_into().unwrap_or(u64::MAX)
}

const fn prompt_mode(mode: PromptMode) -> &'static str {
    match mode {
        PromptMode::Normal => "normal",
        PromptMode::Steer => "steer",
        PromptMode::FollowUp => "follow_up",
    }
}

fn parse_prompt_mode(mode: &str) -> PromptMode {
    match mode {
        "steer" => PromptMode::Steer,
        "follow_up" => PromptMode::FollowUp,
        _ => PromptMode::Normal,
    }
}

fn usize_to_i64(value: usize) -> i64 {
    value.try_into().unwrap_or(i64::MAX)
}
