use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rusqlite::{
    Connection, ErrorCode, OptionalExtension as _, Transaction, TransactionBehavior, params,
};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    agents::{PromptPresentation, QueuedPrompt},
    projects::{DraftSession, Registry},
    protocol::{PromptImage, PromptMode},
    sessions::{SessionSummary, UsageSummary},
};

mod composer;
mod identity;
mod migrate_legacy;
mod migrate_v12;
mod projects;
mod prompts;
mod schema;
mod sessions;
mod settings;
mod traits;

use identity::{bind_locator, ensure_locator_session, ensure_project, target_for_session};

const SCHEMA_VERSION: i64 = 13;
const DATABASE_BUSY_TIMEOUT: Duration = Duration::from_secs(10);
const LEGACY_PI_GPUI_IMPORT_KEY: &str = "legacy_pi_gpui_state_imported";
const REPOSITORY_BACKENDS: [&str; 3] = ["auto", "git", "jj"];

pub(crate) struct StateStore {
    connection: Connection,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub(crate) struct WindowPlacement {
    pub bounds: [f32; 4],
    pub display_uuid: Option<String>,
    pub display_origin: [f32; 2],
    pub state: WindowState,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub(crate) enum WindowState {
    Windowed,
    Maximized,
    Fullscreen,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub(crate) struct CachedConfigurationCatalog {
    pub harness: String,
    pub project: PathBuf,
    pub catalog: crate::agents::ConfigurationCatalog,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub(crate) struct CachedSessionControlDefaults {
    pub harness: String,
    pub model: Option<crate::protocol::Model>,
    pub effort: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ComposerRecord {
    pub target: String,
    pub text: String,
    pub cursor: usize,
    pub selection_start: usize,
    pub selection_end: usize,
    pub history: Vec<String>,
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
