use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use farcaster_projects::{AppliedTrust, StartupTrust, TrustChoice};
use farcaster_sessions::{LoadedHistory, SessionSummary, SessionTransfer};

use super::{
    antigravity::AntigravityAdapter, claude::ClaudeAdapter, codex::CodexAdapter,
    cursor::CursorAdapter, main_session, opencode::OpenCodeAdapter, pi::PiAdapter,
    queued_session::SteeringBoundary,
};
use crate::{
    AgentLaunchConfig, Backend, ConfigurationCatalog, DiscoveredHistory, DiscoveredSession,
    SessionLaunch, SessionStart, SessionTransport, WorkerSessionFactory,
    contract::AgentBackendDescriptor,
};

pub(super) trait BackendAdapter: Sync {
    fn descriptor(&self) -> AgentBackendDescriptor;
    fn launch_configuration(&self, config: &AgentLaunchConfig) -> AgentLaunchConfig;
    fn worker_factory(&self, config: AgentLaunchConfig) -> Arc<dyn WorkerSessionFactory>;
    fn profile_data_environment_key(&self) -> Option<&'static str> {
        None
    }
    fn configuration_catalog(
        &self,
        config: &AgentLaunchConfig,
        project: &Path,
    ) -> Result<ConfigurationCatalog, String>;
    fn spawn(
        &self,
        config: &AgentLaunchConfig,
        launch: SessionLaunch,
    ) -> Result<Box<dyn SessionTransport>, String>;
    fn steering_boundary(&self) -> SteeringBoundary;
    fn supports_auto_title_generation(&self) -> bool {
        false
    }
    fn title_model(
        &self,
        _catalog: &ConfigurationCatalog,
        _active_model: Option<&crate::extensions::Model>,
    ) -> Option<crate::extensions::Model> {
        None
    }
    fn generate_title(
        &self,
        _config: &AgentLaunchConfig,
        _project: &Path,
        _first_prompt: &str,
        _selection: Option<&crate::extensions::Model>,
        _effort: Option<String>,
    ) -> Result<String, String> {
        Err(format!(
            "{} does not expose ephemeral inference",
            self.descriptor().id
        ))
    }
    fn access_modes(
        &self,
        model: Option<&crate::extensions::Model>,
        _sandbox_adapter: Option<&str>,
    ) -> Vec<crate::HarnessAccessMode> {
        let configuration = self.descriptor().capabilities.configuration;
        let declared = model.and_then(|model| model.access_modes.as_deref());
        configuration
            .access_modes
            .iter()
            .copied()
            .filter(|mode| {
                declared.map_or(
                    !configuration.model_required_access_modes.contains(mode),
                    |modes| modes.contains(mode),
                )
            })
            .collect()
    }
    fn supports_sandbox_discovery(&self) -> bool {
        false
    }
    fn annotate_history_message(&self, _message: &mut serde_json::Value) {}
    fn trust_description(&self) -> Option<&'static str> {
        None
    }
    fn project_trust(&self, _project: &Path) -> Result<StartupTrust, String> {
        Ok(StartupTrust::Ready)
    }
    fn apply_project_trust(
        &self,
        _project: &Path,
        _choice: TrustChoice,
    ) -> Result<AppliedTrust, String> {
        Err(format!(
            "{} manages its own project trust",
            self.descriptor().id
        ))
    }
    fn saved_project_trust(&self, _project: &Path) -> Result<Option<(PathBuf, bool)>, String> {
        Ok(None)
    }

    fn rename_session(
        &self,
        _config: &AgentLaunchConfig,
        _project: &Path,
        _session: &Path,
        _id: &str,
        _name: &str,
    ) -> Result<(), String> {
        Err(format!(
            "unsupported session harness: {}",
            self.descriptor().id
        ))
    }
    fn discover(&self, _root: &Path, _query: &str) -> Result<Vec<DiscoveredSession>, String> {
        Err(format!(
            "unsupported session harness: {}",
            self.descriptor().id
        ))
    }
    fn discover_sessions(
        &self,
        root: Option<&Path>,
        query: &str,
    ) -> Result<Vec<SessionSummary>, String> {
        let root = root.ok_or_else(|| "session locator root is unavailable".to_owned())?;
        self.discover(root, query).map(|sessions| {
            sessions
                .into_iter()
                .map(super::session_storage::import_session)
                .collect()
        })
    }
    fn discover_sessions_for_profile(
        &self,
        _config: &AgentLaunchConfig,
        root: Option<&Path>,
        query: &str,
    ) -> Result<Vec<SessionSummary>, String> {
        self.discover_sessions(root, query)
    }
    fn external_history(&self, _path: &Path, _project: &Path) -> Result<DiscoveredHistory, String> {
        Err(format!(
            "unsupported session harness: {}",
            self.descriptor().id
        ))
    }
    fn load_history(&self, path: &Path, project: &Path) -> Result<LoadedHistory, String> {
        let history = self.external_history(path, project)?;
        Ok(LoadedHistory {
            messages: history.messages,
            model: history.model,
            thinking_level: history.thinking_level,
            pending_question: None,
            prompt_deliveries: history.prompt_deliveries,
        })
    }
    fn load_history_for_profile(
        &self,
        _config: &AgentLaunchConfig,
        path: &Path,
        project: &Path,
    ) -> Result<LoadedHistory, String> {
        self.load_history(path, project)
    }
    fn move_family(
        &self,
        _family: &[SessionSummary],
        _project: &Path,
    ) -> Result<SessionTransfer, String> {
        Err(format!(
            "unsupported session move harness: {}",
            self.descriptor().id
        ))
    }
    fn move_family_with_config(
        &self,
        _config: &AgentLaunchConfig,
        family: &[SessionSummary],
        project: &Path,
    ) -> Result<SessionTransfer, String> {
        self.move_family(family, project)
    }
    fn delete_session(&self, _id: &str, _path: &Path) -> Result<Option<PathBuf>, String> {
        Err(format!(
            "Session deletion is not supported for {}",
            self.descriptor().id
        ))
    }
    fn delete_session_with_config(
        &self,
        _config: &AgentLaunchConfig,
        id: &str,
        path: &Path,
    ) -> Result<Option<PathBuf>, String> {
        self.delete_session(id, path)
    }
    fn validate_locator(&self, path: &Path) -> Result<Option<String>, String> {
        let backend = self.descriptor().id;
        main_session::external_session_locator(backend, path)
            .map(Some)
            .ok_or_else(|| format!("session locator does not belong to {backend}"))
    }
    fn validate_launch_locator(&self, launch: &SessionLaunch) -> Result<(), String> {
        if let SessionStart::Resume(path) | SessionStart::Fork(path) = &launch.start {
            self.validate_locator(path)?;
        }
        Ok(())
    }
}

pub(super) fn for_backend(backend: Backend) -> &'static dyn BackendAdapter {
    static PI: PiAdapter = PiAdapter;
    static CODEX: CodexAdapter = CodexAdapter;
    static CURSOR: CursorAdapter = CursorAdapter;
    static OPENCODE: OpenCodeAdapter = OpenCodeAdapter;
    static CLAUDE: ClaudeAdapter = ClaudeAdapter;
    static ANTIGRAVITY: AntigravityAdapter = AntigravityAdapter;
    match backend {
        Backend::Pi => &PI,
        Backend::Codex => &CODEX,
        Backend::Cursor => &CURSOR,
        Backend::OpenCode => &OPENCODE,
        Backend::Claude => &CLAUDE,
        Backend::Antigravity => &ANTIGRAVITY,
    }
}

pub(super) fn program(config: &AgentLaunchConfig, path: PathBuf) -> AgentLaunchConfig {
    let mut config = config.clone();
    config.program = path;
    config
}

pub(super) fn worker_transport(
    config: &AgentLaunchConfig,
    launch: &SessionLaunch,
    worker: Box<dyn crate::WorkerSession>,
    locator: String,
    metadata: main_session::MainSessionMetadata,
    history: Option<DiscoveredHistory>,
) -> Result<Box<dyn SessionTransport>, String> {
    let locator_root = config.locator_root();
    let root = locator_root
        .as_deref()
        .ok_or_else(|| "agent session locator root is not configured".to_owned())?;
    main_session::WorkerSessionTransport::new(
        root,
        launch.harness,
        locator,
        worker,
        metadata,
        history,
    )
    .map(|transport| Box::new(transport) as _)
}

pub(super) fn launch_history(
    launch: &SessionLaunch,
    load: impl FnOnce(&Path) -> Result<DiscoveredHistory, String>,
) -> Result<Option<DiscoveredHistory>, String> {
    match &launch.start {
        SessionStart::New => Ok(None),
        SessionStart::Resume(path) | SessionStart::Fork(path) => load(path).map(Some),
    }
}

pub(super) fn reject_acp_fork(launch: &SessionLaunch) -> Result<(), String> {
    if matches!(launch.start, SessionStart::Fork(_)) {
        Err(format!(
            "{} ACP session fork is not supported",
            launch.harness
        ))
    } else {
        Ok(())
    }
}
