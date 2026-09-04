//! UI-neutral application runtime and active-session ownership.

mod access_mode;
mod catalog;
mod commands;
mod documents;
mod history;
mod process;
mod projection;
mod prompts;
mod session_controls;
mod session_identity;
mod session_loop;
mod status;

pub(crate) use crate::agents::HarnessAccessMode;
use access_mode::AccessModeChangeState;
use history::{annotate_history_presentations, import_agent_session};
#[cfg(test)]
use process::startup_commands;
use process::{can_send_prompt, conversation_mut, reset_snapshot_for_process};
#[cfg(test)]
use projection::stable_session_stats;
use projection::{
    historical_context_stats, update_context_from_event, update_session_goal_from_event,
};
use prompts::DeferredPrompt;
use session_loop::run;
use status::{
    failure_details, failure_summary, notification_target, run_status, semantic_status,
    session_badge_status, tool_starts_worker,
};

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant, SystemTime},
};

use serde_json::{Value, json};

#[cfg(test)]
use crate::app::views::transcript::conversation::TranscriptKind;
use crate::{
    agent_activity::AgentActivity,
    agents::{
        self, AgentLaunchConfig, SessionActivityKind, SessionCommand, SessionEvent, SessionLaunch,
        SessionOperation, SessionStart, SessionTransport,
    },
    app::infrastructure::persistence::StateStore,
    app::views::transcript::conversation::{
        ConversationState, TranscriptItem, annotate_prompt_presentations,
    },
    protocol::{
        AgentMode, ExtensionUiRequest, ExtensionUiResponse, Model, PromptImage, PromptMode,
        SessionState, SlashCommand,
    },
    sessions::{
        self, ExternalActivityTracker, LoadedHistory, SessionDiscovery, SessionSummary,
        SessionWatchEvent, SessionWatcher, TransferMember, archived_root_family_for_path,
        configured_session_root, project_display_history, session_family_for_path,
    },
};
use session_controls::PendingSessionControls;
use session_identity::HarnessConfigurationStore;

const COALESCED_SESSION_REFRESH_DELAY: Duration = Duration::from_millis(100);
const STREAM_PUBLISH_INTERVAL: Duration = Duration::from_millis(16);
const MAX_FAILURE_DETAILS_CHARS: usize = 12_000;
const MAX_FAILURE_SUMMARY_CHARS: usize = 240;

mod supervisor;
mod types;

#[cfg(test)]
use documents::reconcile_live_session_documents;
pub(crate) use supervisor::RuntimeHandle;
use supervisor::{SessionEventSender, SessionRuntimeHandle};
#[cfg(test)]
use supervisor::{
    SupervisorSessionAction, UiEventSender, actor_key_for_command, changed_external_documents,
    command_targets_catalog, initial_draft_command, is_view_only_selection,
    publish_session_status_if_changed, route_session_discovery, rpc_owned_session_paths,
    target_command_needs_actor_message,
};
pub(crate) use types::{ConfigurationStatus, RuntimeCommand, RuntimeEvent, RuntimeSnapshot};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SnapshotChange {
    None,
    Streaming,
    Immediate,
}

struct RuntimeOwner {
    project: PathBuf,
    harness: String,
    session_id: Option<String>,
    process_command: AgentLaunchConfig,
    process: Option<Box<dyn SessionTransport>>,
    snapshot: RuntimeSnapshot,
    owns_session_catalog: bool,
    session_generation: u64,
    session_discovery_in_flight: bool,
    session_refresh_pending: bool,
    session_refresh_due: Option<Instant>,
    process_generation: u64,
    pending_prompt_id: Option<String>,
    pending_prompt_target: Option<String>,
    pending_prompt_item: Option<Arc<TranscriptItem>>,
    pending_outbox_id: Option<i64>,
    title_generation: SessionTitleGeneration,
    transcript_changed_from: Option<usize>,
    event_tx: SessionEventSender,
    discovery_tx: mpsc::Sender<DiscoveryResult>,
    history_tx: mpsc::Sender<HistoryResult>,
    history_generation: u64,
    history_selection_generation: Option<u64>,
    document_refresh_generation: Option<u64>,
    pending_document_refresh: Option<(PathBuf, PathBuf)>,
    active_session: Option<PathBuf>,
    parked_snapshot: Option<RuntimeSnapshot>,
    deferred_prompt: Option<DeferredPrompt>,
    pending_session_controls: PendingSessionControls,
    access_mode_changes: AccessModeChangeState,
    startup_state_loaded: bool,
    startup_history_loaded: bool,
    state: Option<StateStore>,
    session_query: String,
}

struct DiscoveryResult {
    generation: u64,
    result: Result<SessionDiscovery, String>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum HistoryLoadKind {
    Selection,
    DocumentRefresh,
}

struct HistoryResult {
    generation: u64,
    path: PathBuf,
    project: PathBuf,
    kind: HistoryLoadKind,
    result: Result<LoadedHistory, String>,
}

struct SessionTitleResult {
    generation: u64,
    revision: u64,
    result: Result<String, String>,
}

struct SessionTitleGeneration {
    in_flight: bool,
    revision: u64,
    sender: mpsc::Sender<SessionTitleResult>,
    receiver: mpsc::Receiver<SessionTitleResult>,
}

impl Default for SessionTitleGeneration {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            in_flight: false,
            revision: 0,
            sender,
            receiver,
        }
    }
}

#[cfg(test)]
mod tests;
