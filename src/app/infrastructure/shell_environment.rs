use std::{path::Path, process::Command};

const APP_ENV_IMPORTED: &str = "FARCASTER_SHELL_ENV_IMPORTED";

pub(crate) fn import() -> Result<(), String> {
    use std::os::unix::process::CommandExt as _;

    if std::env::var(APP_ENV_IMPORTED).as_deref() == Ok("1") {
        return Ok(());
    }
    let environment = crate::agents::app_shell_environment()?;
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
}
