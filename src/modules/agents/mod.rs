mod adapter;
mod contract;
mod core;

pub(crate) use adapter::project_shell_environment;
pub(crate) use adapter::{
    annotate_history_message, app_shell_environment, apply_project_trust, available_access_modes,
    backend_display_name, backend_statuses, default_login_shell, delete_session_family,
    discover_sessions_for, external_session_identity, generate_session_title,
    load_configuration_catalog, load_session_history, move_session_family, project_trust,
    project_trust_description, rename_session, saved_project_trust, spawn_session,
    supports_auto_title_generation, supports_reasoning_effort, supports_sandbox_discovery,
    supports_session_fork, supports_session_move, supports_startup_command, supports_steering,
    validate_launch, validate_session_move, worker_factories,
};
pub(crate) use contract::extensions;
pub(crate) use contract::{
    AgentLaunchConfig, ConfigurationCatalog, DiscoveredHistory, DiscoveredSession, DiscoveredUsage,
    HarnessAccessMode, PeerMessage, PromptPresentation, QueuedPrompt, SandboxState,
    SessionActivityKind, SessionCommand, SessionContextUsage, SessionEvent, SessionGoal,
    SessionHistory, SessionLaunch, SessionMetadata, SessionOperation, SessionResponse,
    SessionResponsePayload, SessionStart, SessionTransport, SessionUsage, SessionUsageTokens,
    StartWorker, WorkerContext, WorkerInput, WorkerInputResponse, WorkerSnapshot, WorkerStatus,
    valid_worker_name,
};
pub(crate) use core::{
    CallerContext, CallerProfile, CallerRegistry, ChildSessionOutcome, CommonTool, PromptStore,
    TokenUsage, ToolCategory, ToolMetadata, ToolReviewState, WorkerActivity, WorkerActivityState,
    WorkerAssignment, WorkerEvent, WorkerExecution, WorkerFamilyLink, WorkerLaunch, WorkerPool,
    WorkerProfile, WorkerProfiles, WorkerSendMode, WorkerSession, WorkerSessionFactory,
    WorkerUsage, begin_prompt, complete_prompt, enqueue_prompt_with_presentation, fail_prompt,
    has_queued_prompts_for, is_child_input_id, queued_prompts,
};

#[cfg(test)]
pub(crate) use core::CallerIdentity;
