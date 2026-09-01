mod catalog;
mod connection;
mod translate;
mod wire;
mod worker;

use std::path::PathBuf;

pub(in crate::modules::agents::adapter) use catalog::{discover, load_history};
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
}

impl AcpProfile {
    pub(super) fn program(&self) -> PathBuf {
        std::env::var_os(self.path_environment)
            .map(PathBuf::from)
            .unwrap_or_else(|| self.command.into())
    }
}
