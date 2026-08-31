use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug)]
pub(crate) struct SandboxRuntime {
    pub(in crate::modules::access) kind: SandboxRuntimeKind,
}

#[derive(Clone, Debug)]
pub(in crate::modules::access) enum SandboxRuntimeKind {
    Fixed(PathBuf),
    Unavailable,
    #[cfg(test)]
    TestBypass,
}

impl SandboxRuntime {
    pub(crate) fn fixed(program: PathBuf) -> Self {
        Self {
            kind: SandboxRuntimeKind::Fixed(program),
        }
    }

    pub(in crate::modules::access) const fn unavailable() -> Self {
        Self {
            kind: SandboxRuntimeKind::Unavailable,
        }
    }

    #[cfg(test)]
    pub(crate) const fn test_bypass() -> Self {
        Self {
            kind: SandboxRuntimeKind::TestBypass,
        }
    }
}

pub(crate) struct SandboxPaths<'a> {
    pub(crate) project: &'a Path,
    pub(crate) home: &'a Path,
    pub(crate) agent_state: &'a Path,
    pub(crate) temporary: &'a Path,
    pub(crate) metadata_read: &'a [PathBuf],
}

pub(crate) struct SandboxedCommand {
    pub(crate) command: Command,
    pub(in crate::modules::access) _profile: Option<tempfile::NamedTempFile>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FilesystemAccess {
    ReadOnly,
    Sandboxed,
    Full,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NetworkAccess {
    Sandboxed,
    Full,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AccessPolicy {
    pub(crate) filesystem: FilesystemAccess,
    pub(crate) network: NetworkAccess,
}

impl AccessPolicy {
    pub(crate) const fn unrestricted(self) -> bool {
        matches!(self.filesystem, FilesystemAccess::Full)
            && matches!(self.network, NetworkAccess::Full)
    }
}

#[derive(Clone, Default, Eq, PartialEq)]
pub(crate) struct NetworkConfiguration {
    pub(crate) proxy_hosts: Vec<String>,
    pub(crate) proxy_loopback_ports: Vec<u16>,
    pub(crate) app_proxy: Option<String>,
    pub(crate) tls_ca_env_vars: Vec<String>,
}

impl std::fmt::Debug for NetworkConfiguration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NetworkConfiguration")
            .field("proxy_hosts", &self.proxy_hosts)
            .field("proxy_loopback_ports", &self.proxy_loopback_ports)
            .field("app_proxy", &self.app_proxy.as_ref().map(|_| "<redacted>"))
            .field("tls_ca_env_vars", &self.tls_ca_env_vars)
            .finish()
    }
}
