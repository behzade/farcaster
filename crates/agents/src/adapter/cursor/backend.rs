use super::super::{
    backend::{BackendAdapter, program, reject_acp_fork, worker_transport},
    queued_session::SteeringBoundary,
};
use crate::{
    AgentLaunchConfig, ConfigurationCatalog, DiscoveredHistory, DiscoveredSession, SessionLaunch,
    SessionTransport, WorkerSessionFactory, contract::AgentBackendDescriptor,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub(in crate::adapter) struct CursorAdapter;

impl BackendAdapter for CursorAdapter {
    fn descriptor(&self) -> AgentBackendDescriptor {
        super::descriptor()
    }
    fn launch_configuration(&self, config: &AgentLaunchConfig) -> AgentLaunchConfig {
        program(config, super::PROFILE.program())
    }
    fn worker_factory(&self, config: AgentLaunchConfig) -> Arc<dyn WorkerSessionFactory> {
        Arc::new(super::worker_factory(config))
    }
    fn configuration_catalog(
        &self,
        _config: &AgentLaunchConfig,
        project: &Path,
    ) -> Result<ConfigurationCatalog, String> {
        super::load_configuration(project).and_then(super::super::configuration_catalog)
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
        let (worker, locator, metadata, history) = super::spawn_main(&command, &launch)?;
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
    fn delete_session(&self, id: &str, _path: &Path) -> Result<Option<PathBuf>, String> {
        super::delete_session(id).map(|_| None)
    }
}
