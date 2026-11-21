use std::path::{Path, PathBuf};

use crate::AgentLaunchConfig;

impl AgentLaunchConfig {
    pub(super) fn for_worker(
        &self,
        backend: crate::Backend,
        launch: &crate::WorkerLaunch,
    ) -> Result<Self, String> {
        let mut command = self.clone();
        command.profile_id = launch.harness_profile_id.clone();
        command.access_mode = launch.access_mode;
        command.app_proxy = launch.app_proxy.clone();
        command.prompt_boundary_url = None;
        command.validate_profile_backend(backend)?;
        Ok(command)
    }

    pub fn locator_root(&self) -> Option<PathBuf> {
        let root = self.session_locator_root.as_ref()?;
        Some(
            self.profile_id
                .as_ref()
                .map_or_else(|| root.clone(), |id| root.join("profiles").join(id)),
        )
    }

    pub fn selected_profile(&self) -> Result<Option<crate::HarnessProfile>, String> {
        self.profile_id
            .as_deref()
            .map(|id| self.profiles.get(id))
            .transpose()
    }

    pub fn validate_profile_backend(&self, backend: crate::Backend) -> Result<(), String> {
        if let Some(profile) = self.selected_profile()?
            && profile.backend != backend
        {
            return Err(format!(
                "{} uses {}, not {}",
                profile.name, profile.backend, backend
            ));
        }
        Ok(())
    }

    #[doc(hidden)]
    pub fn test_script(script: &Path, mut arguments: Vec<String>) -> Self {
        let mut prefix_args = Vec::with_capacity(arguments.len() + 1);
        prefix_args.push(script.to_string_lossy().into_owned());
        prefix_args.append(&mut arguments);
        Self {
            program: PathBuf::from("sh"),
            prefix_args,
            access_mode: crate::HarnessAccessMode::default(),
            app_proxy: None,
            session_locator_root: None,
            prompt_boundary_url: None,
            profiles: Default::default(),
            profile_id: None,
        }
    }

    pub fn command(&self, project: &Path) -> Result<std::process::Command, String> {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        let environment = super::shell_environment::project_shell_environment(project)?;
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let environment: Option<Vec<(std::ffi::OsString, std::ffi::OsString)>> = None;
        self.command_with_environment(project, environment)
    }

    /// Reconstruct the command only if its environment is available without a shell launch.
    pub(super) fn cached_command(&self, project: &Path) -> Option<std::process::Command> {
        #[cfg(all(not(test), any(target_os = "linux", target_os = "macos")))]
        let environment = Some(super::shell_environment::cached_environment(project)?);
        #[cfg(any(test, not(any(target_os = "linux", target_os = "macos"))))]
        let environment = None;
        self.command_with_environment(project, environment).ok()
    }

    fn command_with_environment(
        &self,
        project: &Path,
        environment: Option<Vec<(std::ffi::OsString, std::ffi::OsString)>>,
    ) -> Result<std::process::Command, String> {
        let profile = self.selected_profile()?;
        let network = farcaster_access::network_configuration(
            environment.as_deref(),
            self.app_proxy.as_deref(),
        );
        let mut environment = environment.unwrap_or_else(|| std::env::vars_os().collect());
        farcaster_access::append_app_proxy_environment(&mut environment, &network);
        let executable = profile.as_ref().map_or(self.program.as_path(), |profile| {
            profile.executable.as_path()
        });
        let search_path = environment
            .iter()
            .find(|(key, _)| key == "PATH")
            .map(|(_, value)| value.as_os_str());
        let program = resolve_agent_program(executable, project, search_path)?;
        let mut command = std::process::Command::new(program);
        // Keep the complete child environment explicit so discovery sees exactly
        // what the backend will receive, including variables that are absent.
        command
            .args(&self.prefix_args)
            .current_dir(project)
            .env_clear()
            .envs(environment);
        if let Some(profile) = profile
            && let (Some(key), Some(directory)) =
                (profile.data_environment_key(), profile.data_directory)
        {
            command.env(key, directory);
        }
        command.env_remove("FARCASTER_PROMPT_BOUNDARY_URL");
        if let Some(url) = &self.prompt_boundary_url {
            command.env("FARCASTER_PROMPT_BOUNDARY_URL", url);
        }
        Ok(command)
    }
}

/// Read the explicit environment prepared by `AgentLaunchConfig::command`.
pub(super) fn environment(
    command: &std::process::Command,
    name: &str,
) -> Option<std::ffi::OsString> {
    command
        .get_envs()
        .find(|(key, _)| *key == name)
        .and_then(|(_, value)| value.map(std::ffi::OsString::from))
}

#[cfg(test)]
#[path = "process_command_tests.rs"]
mod tests;

pub(super) fn resolve_agent_program(
    program: &Path,
    working_directory: &Path,
    search_path: Option<&std::ffi::OsStr>,
) -> Result<PathBuf, String> {
    if program.as_os_str().is_empty() {
        return Err("agent executable is not configured".into());
    }
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
