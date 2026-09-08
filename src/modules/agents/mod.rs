mod adapter;
mod contract;
mod core;

pub(crate) use adapter::{
    annotate_history_message, app_shell_environment, apply_project_trust, backend_display_name,
    backend_statuses, default_login_shell, delete_session_family, discover_sessions_for,
    external_session_identity, generate_session_title, load_configuration_catalog,
    load_session_history, move_session_family, normalize_access_mode, project_trust,
    project_trust_description, rename_session, saved_project_trust, spawn_session,
    supported_access_modes, supports_auto_title_generation, supports_reasoning_effort,
    supports_session_fork, supports_session_move, supports_startup_command, validate_launch,
    validate_session_move, worker_factories,
};
pub(crate) use contract::extensions;
pub(crate) use contract::{
    AgentLaunchConfig, ConfigurationCatalog, DiscoveredHistory, DiscoveredSession, DiscoveredUsage,
    HarnessAccessMode, PeerMessage, PromptPresentation, QueuedPrompt, SessionActivityKind,
    SessionCommand, SessionEvent, SessionGoal, SessionLaunch, SessionMetadata, SessionOperation,
    SessionResponse, SessionStart, SessionTransport, StartWorker, WorkerContext, WorkerInput,
    WorkerInputResponse, valid_worker_name,
};
pub(crate) use core::{
    CallerContext, CallerProfile, CallerRegistry, CommonTool, PromptStore, TokenUsage,
    ToolCategory, ToolMetadata, ToolReviewState, WorkerActivity, WorkerActivityState,
    WorkerAssignment, WorkerEvent, WorkerExecution, WorkerFamilyLink, WorkerLaunch, WorkerPool,
    WorkerProfile, WorkerProfiles, WorkerSendMode, WorkerSession, WorkerSessionFactory,
    WorkerUsage, begin_prompt, complete_prompt, enqueue_prompt_with_presentation, fail_prompt,
    has_queued_prompts_for, is_child_input_id, queued_prompts,
};

#[cfg(test)]
pub(crate) use core::CallerIdentity;
