mod access_mode;
mod catalog;
mod command_queue;
mod commands;
mod documents;
mod history;
mod notifications;
mod process;
mod projection;
mod prompts;
mod session_controls;
mod session_identity;
mod session_loop;
mod status;

pub(crate) use crate::agents::HarnessAccessMode;
use access_mode::AccessModeChangeState;
use history::annotate_history_presentations;
use notifications::interaction_notification;
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
    session_badge_status,
};

use std::{
    collections::{HashMap, HashSet, VecDeque},
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
        self, LoadedHistory, SessionSummary, archived_root_family_for_path, session_family_for_path,
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
mod worker_inputs;

#[cfg(test)]
use documents::reconcile_live_session_documents;
pub(crate) use supervisor::RuntimeHandle;
use supervisor::{SessionEventSender, SessionRuntimeHandle};
#[cfg(test)]
use supervisor::{
    SupervisorSessionAction, UiEventSender, actor_key_for_command, changed_external_documents,
    command_targets_catalog, initial_draft_command, is_view_only_selection,
    publish_session_status_if_changed, route_session_discovery, target_command_needs_actor_message,
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
    session_refresh_due: Option<Instant>,
    process_generation: u64,
    pending_prompt_id: Option<String>,
    pending_prompt_target: Option<String>,
    pending_prompt_item: Option<Arc<TranscriptItem>>,
    pending_outbox_id: Option<i64>,
    title_generation: SessionTitleGeneration,
    transcript_changed_from: Option<usize>,
    event_tx: SessionEventSender,
    history_tx: mpsc::Sender<HistoryResult>,
    history_generation: u64,
    history_selection_generation: Option<u64>,
    document_refresh_generation: Option<u64>,
    pending_document_refresh: Option<(PathBuf, PathBuf)>,
    active_session: Option<PathBuf>,
    parked_snapshot: Option<RuntimeSnapshot>,
    deferred_prompt: Option<DeferredPrompt>,
    queued_prompts: VecDeque<crate::agents::QueuedPrompt>,
    normal_prompt_in_flight: bool,
    pending_session_controls: PendingSessionControls,
    access_mode_changes: AccessModeChangeState,
    startup_state_loaded: bool,
    startup_history_loaded: bool,
    state: Option<StateStore>,
    session_query: String,
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
    new_session: bool,
    in_flight: bool,
    revision: u64,
    sender: mpsc::Sender<SessionTitleResult>,
    receiver: mpsc::Receiver<SessionTitleResult>,
}

impl Default for SessionTitleGeneration {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            new_session: false,
            in_flight: false,
            revision: 0,
            sender,
            receiver,
        }
    }
}

#[cfg(test)]
#[path = "outbox_recovery_tests.rs"]
mod outbox_recovery_tests;
#[cfg(test)]
mod tests;
