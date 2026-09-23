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
        let history = launch_history(&launch, super::load_history)?;
        let command = self.launch_configuration(config);
        let (worker, locator, metadata) = super::spawn_main(&command, &launch)?;
        worker_transport(config, &launch, worker, locator, metadata, history)
    }
    fn discover(&self, root: &Path, query: &str) -> Result<Vec<DiscoveredSession>, String> {
        super::discover(root, query)
    }
    fn external_history(&self, path: &Path, _project: &Path) -> Result<DiscoveredHistory, String> {
        super::load_history(path)
    }
}
