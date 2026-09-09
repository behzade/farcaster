pub(super) mod backend;
mod catalog;
mod configuration;
mod connection;
mod cursor_extension;
mod events;
mod translate;
mod worker;

use std::path::PathBuf;

pub(in crate::modules::agents::adapter) use catalog::{
    list_sessions, load_configuration, load_history,
};
pub(in crate::modules::agents::adapter) use worker::{AcpWorkerFactory, spawn_main};

#[derive(Clone, Debug)]
pub(in crate::modules::agents::adapter) struct AcpProfile {
    pub backend: &'static str,
    pub name: &'static str,
    pub command: &'static str,
    pub path_environment: &'static str,
    pub arguments: &'static [&'static str],
    pub auth_method: Option<&'static str>,
    pub force_argument: Option<&'static str>,
    pub resume_method: &'static str,
    /// Native mode ids for supervised and full access, when set over ACP.
    pub permission_modes: Option<(&'static str, &'static str)>,
}

impl AcpProfile {
    pub(super) fn permission_mode(
        &self,
        access: crate::agents::HarnessAccessMode,
    ) -> Option<&'static str> {
        let full = access == crate::agents::HarnessAccessMode::Full;
        self.permission_modes
            .map(|(supervised, unrestricted)| if full { unrestricted } else { supervised })
    }

    pub(super) fn program(&self) -> PathBuf {
        std::env::var_os(self.path_environment)
            .map(PathBuf::from)
            .unwrap_or_else(|| self.command.into())
    }
}
