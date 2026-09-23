use super::super::{
    backend::{BackendAdapter, launch_history, program, worker_transport},
    queued_session::SteeringBoundary,
};
use crate::{
    AgentLaunchConfig, Backend, ConfigurationCatalog, DiscoveredHistory, DiscoveredSession,
    SessionLaunch, SessionTransport, WorkerSessionFactory, contract::AgentBackendDescriptor,
};
use farcaster_sessions::{SessionSummary, SessionTransfer};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub(in crate::adapter) struct CodexAdapter;

impl BackendAdapter for CodexAdapter {
    fn descriptor(&self) -> AgentBackendDescriptor {
        super::descriptor()
    }
    fn launch_configuration(&self, config: &AgentLaunchConfig) -> AgentLaunchConfig {
        program(config, codex_program())
    }
    fn worker_factory(&self, config: AgentLaunchConfig) -> Arc<dyn WorkerSessionFactory> {
        Arc::new(super::CodexWorkerFactory::new(
            self.launch_configuration(&config),
        ))
    }
    fn configuration_catalog(
        &self,
        config: &AgentLaunchConfig,
        project: &Path,
    ) -> Result<ConfigurationCatalog, String> {
        let command = super::super::configuration_launch(config, Backend::Codex)?;
        super::load_configuration(&command, project).and_then(super::super::configuration_catalog)
    }
    fn steering_boundary(&self) -> SteeringBoundary {
        SteeringBoundary::Native
    }
    fn supports_auto_title_generation(&self) -> bool {
        true
    }
    fn title_model(
        &self,
        catalog: &ConfigurationCatalog,
        active_model: Option<&crate::extensions::Model>,
    ) -> Option<crate::extensions::Model> {
        super::super::auxiliary::select_title_model(
            catalog,
            active_model,
            "FARCASTER_CODEX_TITLE_MODEL",
            &["gpt-5.6-luna", "luna", "nano", "mini"],
            false,
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
        super::super::auxiliary::generate_worker_title(
            config,
            Backend::Codex,
            project,
            first_prompt,
            selection,
            effort,
        )
    }
    fn spawn(
        &self,
        config: &AgentLaunchConfig,
        launch: SessionLaunch,
    ) -> Result<Box<dyn SessionTransport>, String> {
        let history = launch_history(&launch, super::load_history)?;
        let command = self.launch_configuration(config);
        let (worker, locator, metadata) = super::spawn_main(&command, &launch)?;
        worker_transport(config, &launch, worker, locator, metadata, history)
    }
    fn rename_session(
        &self,
        _config: &AgentLaunchConfig,
        _project: &Path,
        _session: &Path,
        id: &str,
        name: &str,
    ) -> Result<(), String> {
        super::rename_session(id, name)
    }
    fn discover(&self, root: &Path, query: &str) -> Result<Vec<DiscoveredSession>, String> {
        super::discover(root, query)
    }
    fn external_history(&self, path: &Path, _project: &Path) -> Result<DiscoveredHistory, String> {
        super::load_history(path)
    }
    fn move_family(
        &self,
        family: &[SessionSummary],
        project: &Path,
    ) -> Result<SessionTransfer, String> {
        super::move_family(family, project)
    }
    fn delete_session(&self, id: &str, _path: &Path) -> Result<Option<PathBuf>, String> {
        super::delete_session(id).map(|_| None)
    }
}

fn codex_program() -> PathBuf {
    std::env::var_os("FARCASTER_CODEX_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| "codex".into())
}
