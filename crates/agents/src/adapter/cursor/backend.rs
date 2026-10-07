use super::super::{
    backend::{BackendAdapter, program, worker_transport},
    main_session,
    queued_session::SteeringBoundary,
};
use super::bridge::Bridge;
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
    fn sign_in_required(&self, config: &AgentLaunchConfig) -> bool {
        super::auth::sign_in_required(config, &std::process::Command::new("node"))
    }
    fn supports_sign_in(&self) -> bool {
        true
    }
    fn sign_in(
        &self,
        config: &AgentLaunchConfig,
        project: &Path,
        cancelled: &std::sync::atomic::AtomicBool,
        on_url: &dyn Fn(String),
    ) -> Result<(), String> {
        super::auth::sign_in(
            &self.launch_configuration(config),
            project,
            cancelled,
            on_url,
        )
    }
    fn descriptor(&self) -> AgentBackendDescriptor {
        super::descriptor()
    }
    fn launch_configuration(&self, config: &AgentLaunchConfig) -> AgentLaunchConfig {
        program(config, super::program())
    }
    fn worker_factory(&self, config: AgentLaunchConfig) -> Arc<dyn WorkerSessionFactory> {
        Arc::new(super::worker::Factory(self.launch_configuration(&config)))
    }
    fn catalog_is_complete(&self, catalog: &ConfigurationCatalog) -> bool {
        super::configuration::from_catalog(catalog).is_some()
    }
    fn configuration_catalog(
        &self,
        config: &AgentLaunchConfig,
        project: &Path,
    ) -> Result<ConfigurationCatalog, String> {
        let bridge = Bridge::start(&self.launch_configuration(config), project)?;
        super::configuration::catalog(&bridge.models()?)
    }
    fn steering_boundary(&self) -> SteeringBoundary {
        SteeringBoundary::Native
    }
    fn spawn(
        &self,
        config: &AgentLaunchConfig,
        launch: SessionLaunch,
    ) -> Result<Box<dyn SessionTransport>, String> {
        let command = self.launch_configuration(config);
        let (worker, locator, metadata, history) = super::worker::spawn_main(&command, &launch)?;
        worker_transport(config, &launch, worker, locator, metadata, history)
    }
    fn discover(&self, root: &Path, query: &str) -> Result<Vec<DiscoveredSession>, String> {
        let config = AgentLaunchConfig {
            session_locator_root: Some(root.into()),
            ..Default::default()
        };
        discover(self, &config, root, query)
    }
    fn discover_sessions_for_profile(
        &self,
        config: &AgentLaunchConfig,
        root: Option<&Path>,
        query: &str,
    ) -> Result<Vec<farcaster_sessions::SessionSummary>, String> {
        let root = root.ok_or("Cursor SDK locator root missing")?;
        discover(self, config, root, query).map(|sessions| {
            sessions
                .into_iter()
                .map(super::super::session_storage::import_session)
                .collect()
        })
    }
    fn external_history(&self, path: &Path, project: &Path) -> Result<DiscoveredHistory, String> {
        let config = config_for_path(path)?;
        load(self, &config, path, project)
    }
    fn load_history_for_profile(
        &self,
        config: &AgentLaunchConfig,
        path: &Path,
        project: &Path,
    ) -> Result<farcaster_sessions::LoadedHistory, String> {
        let history = load(self, config, path, project)?;
        Ok(farcaster_sessions::LoadedHistory {
            messages: history.messages,
            model: history.model,
            thinking_level: history.thinking_level,
            pending_question: None,
            prompt_deliveries: history.prompt_deliveries,
        })
    }
    fn delete_session(&self, id: &str, path: &Path) -> Result<Option<PathBuf>, String> {
        self.delete_session_with_config(&config_for_path(path)?, id, path)
    }
    fn delete_session_with_config(
        &self,
        config: &AgentLaunchConfig,
        id: &str,
        _path: &Path,
    ) -> Result<Option<PathBuf>, String> {
        let project = std::env::current_dir().map_err(|e| e.to_string())?;
        let bridge = Bridge::start(&self.launch_configuration(config), &project)?;
        bridge
            .agent("DeleteAgent", serde_json::json!({"agentId":id}))
            .map(|_| None)
    }
}
fn discover(
    adapter: &CursorAdapter,
    config: &AgentLaunchConfig,
    root: &Path,
    query: &str,
) -> Result<Vec<DiscoveredSession>, String> {
    let project = std::env::current_dir().map_err(|e| e.to_string())?;
    let bridge = Bridge::start(&adapter.launch_configuration(config), &project)?;
    super::history::discover(&bridge, root, query)
}
fn load(
    adapter: &CursorAdapter,
    config: &AgentLaunchConfig,
    path: &Path,
    project: &Path,
) -> Result<DiscoveredHistory, String> {
    let id = main_session::external_session_locator(crate::Backend::Cursor, path)
        .ok_or("Invalid Cursor SDK session locator")?;
    let bridge = Bridge::start(&adapter.launch_configuration(config), project)?;
    super::history::load(&bridge, &id, project)
}
fn config_for_path(path: &Path) -> Result<AgentLaunchConfig, String> {
    // external_session_path is <locator-root>/cursor-cli/<encoded-id>.
    let root = path
        .parent()
        .and_then(Path::parent)
        .ok_or("Invalid Cursor SDK locator root")?;
    Ok(AgentLaunchConfig {
        session_locator_root: Some(root.into()),
        ..Default::default()
    })
}
