use super::super::{
    backend::{BackendAdapter, launch_history, program, worker_transport},
    queued_session::SteeringBoundary,
};
use crate::{
    AgentLaunchConfig, Backend, ConfigurationCatalog, DiscoveredHistory, DiscoveredSession,
    SessionLaunch, SessionTransport, WorkerSessionFactory, contract::AgentBackendDescriptor,
};
use std::{path::Path, sync::Arc};

pub(in crate::adapter) struct ClaudeAdapter;

impl BackendAdapter for ClaudeAdapter {
    fn descriptor(&self) -> AgentBackendDescriptor {
        super::descriptor()
    }
    fn profile_data_environment_key(&self) -> Option<&'static str> {
        Some("CLAUDE_CONFIG_DIR")
    }
    fn launch_configuration(&self, config: &AgentLaunchConfig) -> AgentLaunchConfig {
        program(config, super::program())
    }
    fn worker_factory(&self, config: AgentLaunchConfig) -> Arc<dyn WorkerSessionFactory> {
        Arc::new(super::ClaudeWorkerFactory::new(
            self.launch_configuration(&config),
        ))
    }
    fn configuration_catalog(
        &self,
        config: &AgentLaunchConfig,
        project: &Path,
    ) -> Result<ConfigurationCatalog, String> {
        let command = super::super::configuration_launch(config, Backend::Claude)?;
        super::load_configuration(&command, project).and_then(super::super::configuration_catalog)
    }
    fn steering_boundary(&self) -> SteeringBoundary {
        SteeringBoundary::StopAfterBatch
    }
    fn spawn(
        &self,
        config: &AgentLaunchConfig,
        launch: SessionLaunch,
    ) -> Result<Box<dyn SessionTransport>, String> {
        let history = launch_history(&launch, |path| {
            super::catalog::load_history_with_config(config, path)
        })?;
        let command = self.launch_configuration(config);
        let (worker, locator, metadata) = super::spawn_main(&command, &launch)?;
        worker_transport(config, &launch, worker, locator, metadata, history)
    }
    fn discover(&self, root: &Path, query: &str) -> Result<Vec<DiscoveredSession>, String> {
        super::discover(root, query)
    }
    fn discover_sessions_for_profile(
        &self,
        config: &AgentLaunchConfig,
        root: Option<&Path>,
        query: &str,
    ) -> Result<Vec<farcaster_sessions::SessionSummary>, String> {
        let root = root.ok_or("session locator root is unavailable")?;
        super::catalog::discover_with_config(config, root, query).map(|sessions| {
            sessions
                .into_iter()
                .map(super::super::session_storage::import_session)
                .collect()
        })
    }
    fn external_history(&self, path: &Path, _project: &Path) -> Result<DiscoveredHistory, String> {
        super::load_history(path)
    }
    fn load_history_for_profile(
        &self,
        config: &AgentLaunchConfig,
        path: &Path,
        _project: &Path,
    ) -> Result<farcaster_sessions::LoadedHistory, String> {
        let history = super::catalog::load_history_with_config(config, path)?;
        Ok(farcaster_sessions::LoadedHistory {
            messages: history.messages,
            model: history.model,
            thinking_level: history.thinking_level,
            pending_question: None,
            prompt_deliveries: history.prompt_deliveries,
        })
    }
}
