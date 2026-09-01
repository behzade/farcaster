use std::path::{Path, PathBuf};

use crate::agents::AgentLaunchConfig;

impl AgentLaunchConfig {
    #[cfg(test)]
    pub(crate) fn test_script(script: &Path, mut arguments: Vec<String>) -> Self {
        let mut prefix_args = Vec::with_capacity(arguments.len() + 1);
        prefix_args.push(script.to_string_lossy().into_owned());
        prefix_args.append(&mut arguments);
        Self {
            program: PathBuf::from("sh"),
            prefix_args,
            access_mode: crate::agents::HarnessAccessMode::default(),
            app_proxy: None,
            session_locator_root: None,
        }
    }

    pub(in crate::modules::agents) fn command(
        &self,
        project: &Path,
    ) -> Result<std::process::Command, String> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let environment = super::shell_environment::project_shell_environment(project)?;
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let environment: Option<Vec<(std::ffi::OsString, std::ffi::OsString)>> = None;
        let environment_value = |name: &str| {
            environment
                .as_ref()
                .and_then(|values| values.iter().find(|(key, _)| key == name))
                .map(|(_, value)| value.clone())
                .or_else(|| std::env::var_os(name))
        };
        let program =
            resolve_agent_program(&self.program, project, environment_value("PATH").as_deref())?;
        let network =
            crate::access::network_configuration(environment.as_deref(), self.app_proxy.as_deref());
        let mut environment = environment;
        if let Some(environment) = environment.as_mut() {
            crate::access::append_app_proxy_environment(environment, &network);
        }
        let mut command = std::process::Command::new(program);
        command.args(&self.prefix_args).current_dir(project);
        if let Some(environment) = environment {
            command.env_clear().envs(environment);
        }
        Ok(command)
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
