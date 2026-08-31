use std::path::{Path, PathBuf};

use crate::{
    access,
    agents::{FileAccessMode, NetworkAccessMode, PermissionLevel},
};

#[derive(Clone)]
pub(crate) struct AgentProcessCommand {
    pub program: PathBuf,
    pub prefix_args: Vec<String>,
    pub permission_level: PermissionLevel,
    pub nono: access::NonoExecutable,
    pub grants: Option<access::GrantStore>,
    pub app_proxy: Option<String>,
}

impl AgentProcessCommand {
    #[cfg(test)]
    pub(crate) fn test_script(script: &Path, mut arguments: Vec<String>) -> Self {
        let mut prefix_args = Vec::with_capacity(arguments.len() + 1);
        prefix_args.push(script.to_string_lossy().into_owned());
        prefix_args.append(&mut arguments);
        Self {
            program: PathBuf::from("sh"),
            prefix_args,
            permission_level: PermissionLevel::default(),
            nono: access::test_nono_bypass(),
            grants: None,
            app_proxy: None,
        }
    }

    pub(crate) fn command(&self, project: &Path) -> Result<access::PreparedCommand, String> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let mut environment = super::shell_environment::project_shell_environment(project)?;
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let mut environment: Option<Vec<(std::ffi::OsString, std::ffi::OsString)>> = None;
        let environment_value = |name: &str| {
            environment
                .as_ref()
                .and_then(|values| values.iter().find(|(key, _)| key == name))
                .map(|(_, value)| value.clone())
                .or_else(|| std::env::var_os(name))
        };
        let environment_path = |name: &str| environment_value(name).map(PathBuf::from);
        let home = environment_path("HOME")
            .ok_or_else(|| "HOME is required to compile the Farcaster sandbox policy".to_owned())?;
        let agent_state =
            environment_path("PI_CODING_AGENT_DIR").unwrap_or_else(|| home.join(".pi/agent"));
        let temporary = environment_path("TMPDIR").unwrap_or_else(std::env::temp_dir);
        let access = sandbox_access(self.permission_level);
        if let Some(grants) = &self.grants {
            grants.set_access(access.filesystem, access.network);
        }
        let program =
            resolve_agent_program(&self.program, project, environment_value("PATH").as_deref())?;
        let network = access::network_configuration(
            environment.as_deref(),
            self.app_proxy.as_deref(),
            matches!(access.network, access::NetworkAccess::Sandboxed),
        )?;
        if let Some(environment) = environment.as_mut() {
            access::append_app_proxy_environment(environment, &network);
        }
        let mut prepared = access::prepare_command(
            &self.nono,
            &program,
            &self.prefix_args,
            access::PolicyPaths {
                project,
                home: &home,
                agent_state: &agent_state,
                temporary: &temporary,
            },
            access,
            self.grants.as_ref(),
            &network,
        )?;
        prepared.command.current_dir(project);
        if let Some(environment) = environment {
            prepared.command.env_clear().envs(environment);
        }
        Ok(prepared)
    }
}

fn sandbox_access(level: PermissionLevel) -> access::AccessPolicy {
    let filesystem = match level.files {
        FileAccessMode::ReadOnly => access::FilesystemAccess::ReadOnly,
        FileAccessMode::Sandboxed => access::FilesystemAccess::Sandboxed,
        FileAccessMode::Full => access::FilesystemAccess::Full,
    };
    let network = match level.network {
        NetworkAccessMode::Sandboxed => access::NetworkAccess::Sandboxed,
        NetworkAccessMode::Full => access::NetworkAccess::Full,
    };
    access::AccessPolicy {
        filesystem,
        network,
    }
}

pub(super) fn resolve_agent_program(
    program: &Path,
    working_directory: &Path,
    search_path: Option<&std::ffi::OsStr>,
) -> Result<PathBuf, String> {
    let candidate = if program.is_absolute() {
        Some(program.to_owned())
    } else if program
        .parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty())
    {
        Some(working_directory.join(program))
    } else {
        search_path.and_then(|search_path| {
            std::env::split_paths(search_path)
                .map(|directory| directory.join(program))
                .find(|candidate| is_executable_file(candidate))
        })
    };
    let candidate = candidate.ok_or_else(|| {
        format!(
            "agent executable was not found in the captured PATH: {}",
            program.display()
        )
    })?;
    if !is_executable_file(&candidate) {
        return Err(format!(
            "agent executable is not an executable file: {}",
            candidate.display()
        ));
    }
    candidate
        .canonicalize()
        .map_err(|error| format!("resolve agent executable {}: {error}", candidate.display()))
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}
