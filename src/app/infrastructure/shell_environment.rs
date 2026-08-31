use std::{ffi::OsString, path::Path, process::Command};

type Environment = Vec<(OsString, OsString)>;

const APP_ENV_IMPORTED: &str = "FARCASTER_SHELL_ENV_IMPORTED";
const LAUNCH_ENVIRONMENT: [&str; 11] = [
    "FARCASTER_CODEX_PATH",
    "FARCASTER_DATA_DIR",
    "FARCASTER_GIT",
    "FARCASTER_JJ",
    "FARCASTER_NONO_PATH",
    "FARCASTER_NVIM",
    "FARCASTER_OPENCODE_PATH",
    "FARCASTER_PI_PATH",
    "FARCASTER_SHELL",
    "PI_CODING_AGENT_DIR",
    "PI_CODING_AGENT_SESSION_DIR",
];

pub(crate) fn import() -> Result<(), String> {
    use std::os::unix::process::CommandExt as _;

    if std::env::var(APP_ENV_IMPORTED).as_deref() == Ok("1") {
        return Ok(());
    }
    let environment =
        preserve_launch_environment(crate::agents::app_shell_environment()?, |name| {
            std::env::var_os(name)
        });
    let executable = std::env::current_exe()
        .map_err(|error| format!("resolve farcaster executable for shell environment: {error}"))?;
    let error = Command::new(executable)
        .args(std::env::args_os().skip(1))
        .env_clear()
        .envs(environment)
        .env(APP_ENV_IMPORTED, "1")
        .exec();
    Err(format!(
        "relaunch farcaster with the login-shell environment: {error}"
    ))
}

fn preserve_launch_environment(
    mut environment: Environment,
    value: impl Fn(&str) -> Option<OsString>,
) -> Environment {
    for name in LAUNCH_ENVIRONMENT {
        let Some(value) = value(name) else {
            continue;
        };
        environment.retain(|(existing, _)| existing != name);
        environment.push((name.into(), value));
    }
    environment
}

pub(in crate::app) fn terminal_login_shell_command() -> String {
    let shell = std::env::var_os("FARCASTER_SHELL")
        .map(Into::into)
        .unwrap_or_else(crate::agents::default_login_shell);
    login_shell_command(&shell)
}

fn login_shell_command(shell: &Path) -> String {
    format!("'{}' -l", shell.to_string_lossy().replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_shell_command_quotes_the_executable_path() {
        assert_eq!(
            login_shell_command(Path::new("/tmp/my shell's bin")),
            "'/tmp/my shell'\\''s bin' -l"
        );
    }

    #[test]
    fn login_shell_relaunch_preserves_explicit_launch_configuration() {
        let environment = preserve_launch_environment(
            vec![
                ("PATH".into(), "/login/bin".into()),
                ("FARCASTER_NONO_PATH".into(), "/login/nono".into()),
            ],
            |name| (name == "FARCASTER_NONO_PATH").then(|| "/nix/store/nono".into()),
        );

        assert!(environment.contains(&("PATH".into(), "/login/bin".into())));
        assert!(environment.contains(&("FARCASTER_NONO_PATH".into(), "/nix/store/nono".into())));
        assert!(!environment.contains(&("FARCASTER_NONO_PATH".into(), "/login/nono".into())));
    }
}
