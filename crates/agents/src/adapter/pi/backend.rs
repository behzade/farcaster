use super::super::{backend::BackendAdapter, queued_session::SteeringBoundary};
use crate::{
    AgentLaunchConfig, ConfigurationCatalog, SessionLaunch, SessionStart, SessionTransport,
    WorkerSessionFactory, contract::AgentBackendDescriptor,
};
use farcaster_projects::{AppliedTrust, StartupTrust, TrustChoice};
use farcaster_sessions::{LoadedHistory, SessionSummary, SessionTransfer, TransferMember};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub(in crate::adapter) struct PiAdapter;

impl BackendAdapter for PiAdapter {
    fn descriptor(&self) -> AgentBackendDescriptor {
        super::descriptor()
    }
    fn launch_configuration(&self, config: &AgentLaunchConfig) -> AgentLaunchConfig {
        super::launch_configuration(config)
    }
    fn worker_factory(&self, config: AgentLaunchConfig) -> Arc<dyn WorkerSessionFactory> {
        Arc::new(super::PiWorkerFactory::new(config))
    }
    fn configuration_catalog(
        &self,
        config: &AgentLaunchConfig,
        project: &Path,
    ) -> Result<ConfigurationCatalog, String> {
        load_configuration(config, project)
    }
    fn steering_boundary(&self) -> SteeringBoundary {
        SteeringBoundary::Held
    }
    fn supports_auto_title_generation(&self) -> bool {
        true
    }
    fn title_model(
        &self,
        catalog: &ConfigurationCatalog,
        active_model: Option<&crate::extensions::Model>,
    ) -> Option<crate::extensions::Model> {
        let preferences: &[&str] = match active_model.map(|model| model.provider.as_str()) {
            Some("openai-codex" | "openai") => &["gpt-5.6-luna", "luna", "nano", "mini"],
            Some("anthropic") => &["haiku"],
            Some("google") => &["flash-lite", "flash"],
            Some(_) => &["nano", "mini", "small", "lite", "flash"],
            None => &[],
        };
        super::super::auxiliary::select_title_model(
            catalog,
            active_model,
            "FARCASTER_PI_TITLE_MODEL",
            preferences,
            true,
        )
    }
    fn generate_title(
        &self,
        config: &AgentLaunchConfig,
        project: &Path,
        first_prompt: &str,
        selection: Option<&crate::extensions::Model>,
        effort: Option<String>,
    ) -> Result<String, String> {
        super::super::auxiliary::generate_pi_title(
            &super::launch_configuration(config),
            project,
            first_prompt,
            selection,
            effort.as_deref(),
        )
    }
    fn access_modes(
        &self,
        _model: Option<&crate::extensions::Model>,
        sandbox_adapter: Option<&str>,
    ) -> Vec<crate::HarnessAccessMode> {
        super::sandbox::access_modes(sandbox_adapter).to_vec()
    }
    fn supports_sandbox_discovery(&self) -> bool {
        true
    }
    fn annotate_history_message(&self, message: &mut serde_json::Value) {
        super::annotate_history_message(message);
    }
    fn trust_description(&self) -> Option<&'static str> {
        Some(
            "Trusting allows Pi to load project settings and resources, install missing project packages, and execute project extensions.",
        )
    }
    fn project_trust(&self, project: &Path) -> Result<StartupTrust, String> {
        super::trust::startup_trust(project)
    }
    fn apply_project_trust(
        &self,
        project: &Path,
        choice: TrustChoice,
    ) -> Result<AppliedTrust, String> {
        super::trust::apply(project, choice)
    }
    fn saved_project_trust(&self, project: &Path) -> Result<Option<(PathBuf, bool)>, String> {
        super::trust::saved_decision(project)
    }
    fn spawn(
        &self,
        config: &AgentLaunchConfig,
        launch: SessionLaunch,
    ) -> Result<Box<dyn SessionTransport>, String> {
        let process = match &launch.start {
            SessionStart::New => super::PiRpcProcess::spawn_with_optional_waker(
                config,
                &launch.project,
                None,
                launch.wake,
            ),
            SessionStart::Resume(session) => super::PiRpcProcess::spawn_with_optional_waker(
                config,
                &launch.project,
                Some(session),
                launch.wake,
            ),
            SessionStart::Fork(source) => super::PiRpcProcess::spawn_fork_with_optional_waker(
                config,
                &launch.project,
                source,
                launch.wake,
            ),
        }?;
        Ok(Box::new(process))
    }
    fn rename_session(
        &self,
        config: &AgentLaunchConfig,
        project: &Path,
        session: &Path,
        _id: &str,
        name: &str,
    ) -> Result<(), String> {
        super::PiRpcProcess::rename_session(config, project, session, name)
    }
    fn load_history(&self, path: &Path, _project: &Path) -> Result<LoadedHistory, String> {
        super::session_files::load_history(path)
    }
    fn discover_sessions(
        &self,
        _root: Option<&Path>,
        query: &str,
    ) -> Result<Vec<SessionSummary>, String> {
        super::session_files::discover(query).map(|result| result.sessions)
    }
    fn move_family(
        &self,
        family: &[SessionSummary],
        project: &Path,
    ) -> Result<SessionTransfer, String> {
        let root = &family[0];
        let members = family
            .iter()
            .map(|session| TransferMember {
                path: session.path.clone(),
                id: session.id.clone(),
                parent_id: session.parent_session.clone(),
            })
            .collect::<Vec<_>>();
        super::transfer::move_to_project(&members, &root.id, project, &root.path)
    }
    fn validate_locator(&self, path: &Path) -> Result<Option<String>, String> {
        super::session_files::validate_session_file(path).map(|_| None)
    }
    fn validate_launch_locator(&self, _launch: &SessionLaunch) -> Result<(), String> {
        Ok(())
    }
    fn delete_session(&self, _id: &str, path: &Path) -> Result<Option<PathBuf>, String> {
        Ok(Some(path.to_owned()))
    }
}

fn load_configuration(
    config: &AgentLaunchConfig,
    project: &Path,
) -> Result<ConfigurationCatalog, String> {
    use crate::SessionTransport as _;

    let mut process = super::PiRpcProcess::spawn_catalog(config, project)?;
    process.send(crate::SessionCommand::ListModels)?;
    process.send(crate::SessionCommand::ListReasoningLevels)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut catalog = ConfigurationCatalog::default();
    let mut models_loaded = false;
    let mut efforts_loaded = false;
    while std::time::Instant::now() < deadline && !(models_loaded && efforts_loaded) {
        match process.poll() {
            Some(crate::SessionEvent::Response(response)) => {
                match response.result.map_err(|error| error.to_string())? {
                    crate::SessionResponsePayload::ListModels(models) => {
                        catalog.models = models;
                        models_loaded = true;
                    }
                    crate::SessionResponsePayload::ListReasoningLevels(levels) => {
                        catalog.efforts = levels;
                        efforts_loaded = true;
                    }
                    _ => {}
                }
            }
            Some(crate::SessionEvent::Failure(error)) => return Err(error),
            Some(_) => {}
            None => std::thread::sleep(std::time::Duration::from_millis(5)),
        }
    }
    let sandbox_adapter = process.sandbox_adapter().map(str::to_owned);
    let _ = process.close();
    if models_loaded && efforts_loaded {
        catalog.sandbox_adapter = sandbox_adapter;
        Ok(catalog)
    } else {
        Err("timed out loading Pi configuration catalog".into())
    }
}
