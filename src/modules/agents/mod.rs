mod adapter;
mod contract;
mod core;

pub(crate) use adapter::{
    app_shell_environment, backend_display_name, backend_statuses, default_login_shell,
    delete_external_session, discover_external_sessions, external_session_identity,
    generate_session_title, is_external_session, load_configuration_catalog, load_external_history,
    normalize_access_mode, rename_session, spawn_session, supported_access_modes,
    supports_auto_title_generation, supports_startup_command, validate_launch, worker_factories,
};
pub(crate) use contract::extensions;
pub(crate) use contract::{
    AgentLaunchConfig, ConfigurationCatalog, DiscoveredHistory, DiscoveredSession, DiscoveredUsage,
    HarnessAccessMode, PeerMessage, PromptPresentation, QueuedPrompt, SessionActivityKind,
    SessionCommand, SessionEvent, SessionLaunch, SessionOperation, SessionResponse, SessionStart,
    SessionTransport, StartWorker, WorkerContext, WorkerInput, WorkerInputResponse,
    valid_worker_name,
};
pub(crate) use core::{
    CallerContext, CallerRegistry, CommonTool, PromptStore, TokenUsage, ToolReviewState,
    WorkerActivity, WorkerActivityState, WorkerEvent, WorkerLaunch, WorkerPool, WorkerSendMode,
    WorkerSession, WorkerSessionFactory, WorkerUsage, begin_prompt, complete_prompt,
    enqueue_prompt_with_presentation, fail_prompt, has_queued_prompts_for, queued_prompts,
};
