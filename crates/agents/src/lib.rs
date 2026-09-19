extern crate self as agents;

#[cfg(test)]
use farcaster_conversation as conversation;

mod adapter;
pub mod builtin_mcp;
pub use farcaster_agent_protocol as contract;
mod core;

pub use adapter::project_shell_environment;
pub use adapter::{
    annotate_history_message, app_shell_environment, apply_project_trust, available_access_modes,
    backend_display_name, backend_statuses, default_login_shell, delete_session_family,
    discover_sessions_for, effort_label, external_session_identity, generate_session_title,
    load_configuration_catalog, load_session_history, move_session_family, project_trust,
    project_trust_description, rename_session, saved_project_trust, spawn_session,
    supports_auto_title_generation, supports_reasoning_effort, supports_reasoning_reset,
    supports_sandbox_discovery, supports_session_fork, supports_session_move,
    supports_startup_command, supports_steering, validate_launch, validate_session_move,
    worker_factories,
};
#[cfg(test)]
pub use contract::WorkerStatus;
pub use contract::extensions;
pub use contract::{
    AccountUsage, AccountUsageWindow, Backend, ConfigurationCatalog, DiscoveredHistory,
    DiscoveredSession, DiscoveredUsage, HarnessAccessMode, PeerMessage, PromptOutcome,
    PromptPresentation, QueuedPrompt, SandboxState, SessionActivityKind, SessionCommand,
    SessionContextUsage, SessionEvent, SessionGoal, SessionHistory, SessionLaunch, SessionMetadata,
    SessionOperation, SessionResponse, SessionResponseErrorKind, SessionResponsePayload,
    SessionStart, SessionTransport, SessionUsage, SessionUsageTokens, StartWorker, WorkerContext,
    WorkerInput, WorkerInputResponse, WorkerSnapshot, effort_rank, model_efforts,
    valid_worker_name, validate_child_access,
};

#[derive(Clone, Default)]
pub struct AgentLaunchConfig {
    pub program: std::path::PathBuf,
    pub prefix_args: Vec<String>,
    pub access_mode: HarnessAccessMode,
    pub app_proxy: Option<String>,
    pub session_locator_root: Option<std::path::PathBuf>,
}
pub use core::{
    CallerContext, CallerProfile, CallerRegistry, ChildSessionOutcome, CommonTool,
    ExecutionBinding, PromptStore, TokenUsage, ToolCategory, ToolMetadata, ToolReviewState,
    WorkerActivity, WorkerActivityState, WorkerAssignment, WorkerEvent, WorkerExecution,
    WorkerFamilyLink, WorkerLaunch, WorkerModelSelection, WorkerPool, WorkerProfile,
    WorkerProfiles, WorkerRouting, WorkerSendMode, WorkerSession, WorkerSessionFactory,
    WorkerUsage, begin_prompt, complete_prompt_with_receipt, enqueue_prompt_with_presentation,
    fail_prompt, has_queued_prompts_for, is_child_input_id, mark_prompt_delivery_unknown,
    queued_prompts,
};

#[cfg(test)]
pub use adapter::live_tests::support as live_e2e_support;
#[cfg(test)]
pub use core::CallerIdentity;
