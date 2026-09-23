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

pub(in crate::adapter) struct OpenCodeAdapter;

impl BackendAdapter for OpenCodeAdapter {
    fn descriptor(&self) -> AgentBackendDescriptor {
        super::descriptor()
    }
    fn launch_configuration(&self, config: &AgentLaunchConfig) -> AgentLaunchConfig {
        program(config, super::program())
    }
    fn worker_factory(&self, config: AgentLaunchConfig) -> Arc<dyn WorkerSessionFactory> {
        let mut command = self.launch_configuration(&config);
        command.access_mode = crate::HarnessAccessMode::Sandboxed;
        Arc::new(super::OpenCodeWorkerFactory::new(command))
    }
    fn configuration_catalog(
        &self,
        config: &AgentLaunchConfig,
        project: &Path,
    ) -> Result<ConfigurationCatalog, String> {
        let command = super::super::configuration_launch(config, Backend::OpenCode)?;
        super::load_configuration(&command, project).and_then(super::super::configuration_catalog)
    }
    fn steering_boundary(&self) -> SteeringBoundary {
        SteeringBoundary::Held
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
