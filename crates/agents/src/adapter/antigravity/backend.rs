use super::super::{
    acp,
    backend::{BackendAdapter, program, reject_acp_fork, worker_transport},
    queued_session::SteeringBoundary,
};
use crate::{
    AgentLaunchConfig, ConfigurationCatalog, DiscoveredHistory, DiscoveredSession, SessionLaunch,
    SessionTransport, WorkerSessionFactory, contract::AgentBackendDescriptor,
};
use std::{path::Path, sync::Arc};

pub(in crate::adapter) struct AntigravityAdapter;

impl BackendAdapter for AntigravityAdapter {
    fn descriptor(&self) -> AgentBackendDescriptor {
        super::descriptor()
    }
    fn launch_configuration(&self, config: &AgentLaunchConfig) -> AgentLaunchConfig {
        program(config, super::PROFILE.program())
    }
    fn worker_factory(&self, config: AgentLaunchConfig) -> Arc<dyn WorkerSessionFactory> {
        Arc::new(acp::AcpWorkerFactory::new(
            self.launch_configuration(&config),
            super::PROFILE.clone(),
        ))
    }
    fn configuration_catalog(
        &self,
        _config: &AgentLaunchConfig,
        project: &Path,
    ) -> Result<ConfigurationCatalog, String> {
        let (metadata, _) = acp::load_configuration(&super::PROFILE, project)?;
        super::super::configuration_catalog(metadata)
    }
    fn steering_boundary(&self) -> SteeringBoundary {
        SteeringBoundary::Unsupported
    }
    fn spawn(
        &self,
        config: &AgentLaunchConfig,
        launch: SessionLaunch,
    ) -> Result<Box<dyn SessionTransport>, String> {
        reject_acp_fork(&launch)?;
        let command = self.launch_configuration(config);
        let (worker, locator, metadata, history) =
            acp::spawn_main(&command, &super::PROFILE, &launch)?;
        worker_transport(config, &launch, worker, locator, metadata, history)
    }
    fn discover(&self, _root: &Path, _query: &str) -> Result<Vec<DiscoveredSession>, String> {
        Ok(Vec::new())
    }
    fn external_history(&self, path: &Path, project: &Path) -> Result<DiscoveredHistory, String> {
        super::load_history(path, project)
    }
}
