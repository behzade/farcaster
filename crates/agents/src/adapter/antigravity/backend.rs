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
    fn cli_command_hint(&self, name: &str) -> Option<&'static str> {
        matches!(name, "usage" | "quota")
            .then_some("To view your quota, open the Antigravity CLI (agy) and enter /usage.")
    }
    fn terminal_sign_in(
        &self,
        config: &AgentLaunchConfig,
        project: &Path,
        method: &str,
    ) -> Result<Option<std::process::Command>, String> {
        if method != "terminal-login" {
            return Ok(None);
        }
        let mut command = self.launch_configuration(config).command(project)?;
        if !super::integrated_cli(&command) {
            return Err("Terminal sign-in requires the agy CLI".into());
        }
        command.args(["acp", "--app-data-dir", "antigravity-acp", "--login"]);
        Ok(Some(command))
    }
    fn supports_sign_in(&self) -> bool {
        true
    }
    fn sign_in_methods(&self) -> &'static [(&'static str, &'static str)] {
        super::auth::METHODS
    }
    fn sign_in(
        &self,
        config: &AgentLaunchConfig,
        project: &Path,
        method: &str,
        cancelled: &std::sync::atomic::AtomicBool,
        _: &dyn Fn(String),
    ) -> Result<(), String> {
        let command = self.launch_configuration(config).command(project)?;
        super::auth::request(command, Some(method), cancelled)
    }
    fn account_capabilities(&self) -> crate::accounts::AccountCapabilities {
        crate::accounts::AccountCapabilities {
            identity: true,
            login: true,
            logout: true,
            ..Default::default()
        }
    }
    fn logout_account(
        &self,
        config: &AgentLaunchConfig,
        source: &crate::accounts::AccountSource,
        _: &crate::accounts::AccountIdentity,
        project: &Path,
    ) -> Result<(), String> {
        use crate::accounts::*;
        let command = self.launch_configuration(config).command(project)?;
        if super::account_source(config, &command, project).as_ref() != Some(source)
            || !super::auth::capabilities(&command).logout
        {
            return Err(
                "Antigravity credential source changed. Refresh accounts before signing out."
                    .into(),
            );
        }
        super::auth::request(command, None, &false.into())?;
        publish_account_event(AccountEvent::Observed {
            observation: AccountObservation {
                source: source.clone(),
                identity: None,
                auth: AccountAuthState::SignedOut,
                limits: Vec::new(),
                complete_limits: true,
                limits_failed: false,
                observed_at: account_now(),
                capabilities: self.account_capabilities(),
            },
            run_id: None,
            evidence: AccountBindingEvidence::ObservedAtStartup,
        });
        Ok(())
    }
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
        config: &AgentLaunchConfig,
        project: &Path,
    ) -> Result<ConfigurationCatalog, String> {
        let command = self.launch_configuration(config);
        let (mut metadata, _) = acp::load_configuration(&super::PROFILE, &command, project)?;
        if let Some(identity) = metadata.account.take() {
            super::observe_account(config, &command.command(project)?, project, identity);
        }
        super::super::configuration_catalog(metadata)
    }
    fn steering_boundary(&self, config: &AgentLaunchConfig) -> SteeringBoundary {
        let path = config
            .selected_profile()
            .ok()
            .flatten()
            .map(|profile| profile.executable)
            .unwrap_or_else(|| super::PROFILE.program());
        if super::integrated_cli(&std::process::Command::new(path)) {
            SteeringBoundary::Native
        } else {
            SteeringBoundary::Unsupported
        }
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
    fn discover(&self, root: &Path, query: &str) -> Result<Vec<DiscoveredSession>, String> {
        self.list(&Default::default(), root, query)
    }
    fn discover_sessions_for_profile(
        &self,
        config: &AgentLaunchConfig,
        root: Option<&Path>,
        query: &str,
    ) -> Result<Vec<farcaster_sessions::SessionSummary>, String> {
        Ok(self
            .list(
                config,
                root.ok_or("Session locator root is unavailable")?,
                query,
            )?
            .into_iter()
            .map(super::super::session_storage::import_session)
            .collect())
    }
    fn external_history(&self, path: &Path, project: &Path) -> Result<DiscoveredHistory, String> {
        super::load_history(
            &self.launch_configuration(&AgentLaunchConfig::default()),
            path,
            project,
        )
    }
    fn load_history_for_profile(
        &self,
        config: &AgentLaunchConfig,
        path: &Path,
        project: &Path,
    ) -> Result<farcaster_sessions::LoadedHistory, String> {
        let history = super::load_history(&self.launch_configuration(config), path, project)?;
        Ok(farcaster_sessions::LoadedHistory {
            messages: history.messages,
            model: history.model,
            thinking_level: history.thinking_level,
            pending_question: None,
            prompt_deliveries: history.prompt_deliveries,
        })
    }
}

impl AntigravityAdapter {
    fn list(
        &self,
        config: &AgentLaunchConfig,
        root: &Path,
        query: &str,
    ) -> Result<Vec<DiscoveredSession>, String> {
        let query = query.to_ascii_lowercase();
        Ok(acp::catalog::list_sessions_with_config(
            &super::PROFILE,
            &self.launch_configuration(config),
        )?
        .iter()
        .filter_map(|s| acp::catalog::listed_session(&super::PROFILE, root, &query, s))
        .collect())
    }
}
