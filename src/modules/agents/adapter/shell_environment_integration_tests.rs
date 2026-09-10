//! Real-process contracts for shell capture. No user shell configuration is sourced.
use super::*;
use std::time::{Duration, Instant};

#[test]
#[ignore = "requires fish and script; runs real interactive login-shell capture"]
fn real_fish_capture_preserves_launch_and_first_prompt_environment() -> TestResult {
    check_shell_capture("fish")
}

#[test]
#[ignore = "requires bash and script; runs real interactive login-shell capture"]
fn real_bash_capture_preserves_launch_and_first_prompt_environment() -> TestResult {
    check_shell_capture("bash")
}

#[test]
#[ignore = "requires zsh and script; runs real interactive login-shell capture"]
fn real_zsh_capture_preserves_launch_and_first_prompt_environment() -> TestResult {
    check_shell_capture("zsh")
}

fn check_shell_capture(shell_name: &str) -> TestResult {
    let lookup = Command::new("/bin/sh")
        .args(["-c", "command -v \"$1\"", "lookup", shell_name])
        .output()?;
    if !lookup.status.success() {
        return Err(
            format!("integration prerequisite missing: {shell_name} must be on PATH").into(),
        );
    }
    let executable_path =
        PathBuf::from(OsString::from_vec(lookup.stdout.trim_ascii_end().to_vec()));
    let temp = tempdir()?;
    let home = temp.path().join("home");
    let config = home.join(".config/fish");
    let project = temp.path().join("project with spaces");
    let bin = project.join("prompt tools");
    fs::create_dir_all(&config)?;
    fs::create_dir_all(&bin)?;
    install_shell_config(shell_name, &home)?;
    let tool = bin.join("farcaster-capture-probe");
    executable(
        &tool,
        "#!/bin/sh\nprintf '%s\\000' \"$CAPTURE_OVERRIDE\" \"$CAPTURE_CONFIG_VALUE\" \"$CAPTURE_PROJECT\"\n",
    )?;
    let shell = temp.path().join("fixture shell");
    let private = if shell_name == "fish" {
        "--private"
    } else {
        ""
    };
    executable(
        &shell,
        &format!(
            "#!/bin/sh\nexport HOME={}\nexport XDG_CONFIG_HOME={}\nexport XDG_DATA_HOME={}\nexport XDG_CACHE_HOME={}\nexport ZDOTDIR=\"$HOME\"\nexport TERM=xterm-256color\nexport CAPTURE_OVERRIDE=inherited\nexec {} {private} \"$@\"\n",
            quote(&home),
            quote(&home.join(".config")),
            quote(&home.join(".local/share")),
            quote(&home.join(".cache")),
            quote(&executable_path),
        ),
    )?;

    let started = Instant::now();
    let environment = capture_login_shell_environment(&shell, &project)?;
    let elapsed = started.elapsed();
    check_environment(environment, &project, &bin)?;
    eprintln!("real {shell_name} environment capture: {elapsed:?}");
    // A regression ceiling, not a benchmark: fish previously waited ten seconds.
    assert!(elapsed < Duration::from_secs(5), "capture took {elapsed:?}");
    Ok(())
}

fn install_shell_config(shell: &str, home: &Path) -> TestResult {
    match shell {
        "fish" => fs::write(
            home.join(".config/fish/config.fish"),
            r#"
if status is-login
    set -gx CAPTURE_LOGIN loaded
end
if status is-interactive
    set -gx CAPTURE_INTERACTIVE loaded
end
if test -t 0; and test -t 1
    set -gx CAPTURE_TTY yes
end
set -gx CAPTURE_CONFIG_VALUE 'left=right
snowman: ☃'
set -gx CAPTURE_EMPTY ''
set -gx CAPTURE_OVERRIDE configured
# Model direnv-style first-prompt exports, including a project-local executable.
function capture_first_prompt --on-event fish_prompt
    set -gx CAPTURE_PROMPT loaded
    set -gx CAPTURE_PROJECT "$PWD"
    set -gx CAPTURE_OVERRIDE prompt
    set -gx PATH "$PWD/prompt tools" $PATH
    printf 'first-prompt hook chatter\n'
end
function fish_prompt
    printf 'fixture> '
end
printf 'configuration chatter\n'
"#,
        )?,
        "bash" => {
            fs::write(
                home.join(".bash_profile"),
                "shopt -q login_shell && export CAPTURE_LOGIN=loaded\n. \"$HOME/.bashrc\"\n",
            )?;
            fs::write(
                home.join(".bashrc"),
                format!("{POSIX_CONFIG}\nPROMPT_COMMAND=capture_first_prompt\n"),
            )?;
        }
        "zsh" => {
            fs::write(
                home.join(".zprofile"),
                "[[ -o login ]] && export CAPTURE_LOGIN=loaded\n",
            )?;
            fs::write(
                home.join(".zshrc"),
                format!("{POSIX_CONFIG}\nprecmd_functions=(capture_first_prompt)\n"),
            )?;
        }
        _ => return Err(format!("unsupported shell fixture: {shell}").into()),
    }
    Ok(())
}

const POSIX_CONFIG: &str = r#"
case "$-" in *i*) export CAPTURE_INTERACTIVE=loaded;; esac
if test -t 0 && test -t 1; then export CAPTURE_TTY=yes; fi
export CAPTURE_CONFIG_VALUE='left=right
snowman: ☃'
export CAPTURE_EMPTY=''
export CAPTURE_OVERRIDE=configured
capture_first_prompt() {
    export CAPTURE_PROMPT=loaded
    export CAPTURE_PROJECT="$PWD"
    export CAPTURE_OVERRIDE=prompt
    export PATH="$PWD/prompt tools:$PATH"
    printf 'first-prompt hook chatter\n'
}
PS1='fixture> '
printf 'configuration chatter\n'
"#;

fn check_environment(environment: Environment, project: &Path, bin: &Path) -> TestResult {
    for (name, expected) in [
        ("CAPTURE_LOGIN", "loaded"),
        ("CAPTURE_INTERACTIVE", "loaded"),
        ("CAPTURE_TTY", "yes"),
        ("CAPTURE_PROMPT", "loaded"),
        ("CAPTURE_OVERRIDE", "prompt"),
        ("CAPTURE_CONFIG_VALUE", "left=right\nsnowman: ☃"),
        ("CAPTURE_EMPTY", ""),
        ("TERM", "xterm-256color"),
    ] {
        assert_eq!(
            value(&environment, name)?,
            OsString::from(expected),
            "{name}"
        );
    }
    assert_eq!(
        fs::canonicalize(PathBuf::from(value(&environment, "CAPTURE_PROJECT")?))?,
        fs::canonicalize(project)?,
    );
    let path = value(&environment, "PATH")?;
    assert_eq!(std::env::split_paths(&path).next().as_deref(), Some(bin));
    let environment = with_project_path_handoff(environment);
    assert_eq!(value(&environment, PROJECT_PATH_HANDOFF)?, path);

    // Consumer boundary: a child with only the captured environment must find
    // the prompt-installed executable and receive its exports intact.
    let child = Command::new("/bin/sh")
        .args(["-c", "exec farcaster-capture-probe"])
        .env_clear()
        .envs(environment)
        .current_dir(&project)
        .output()?;
    assert!(child.status.success(), "probe failed: {:?}", child.stderr);
    let records: Vec<_> = child.stdout.split(|byte| *byte == 0).collect();
    assert_eq!(records.len(), 4);
    assert_eq!(records[0], b"prompt");
    assert_eq!(records[1], "left=right\nsnowman: ☃".as_bytes());
    assert_eq!(
        fs::canonicalize(PathBuf::from(OsString::from_vec(records[2].to_vec())))?,
        fs::canonicalize(project)?,
    );
    assert!(records[3].is_empty());
    Ok(())
}

fn value(environment: &Environment, name: &str) -> Result<OsString, Box<dyn Error>> {
    environment
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.clone())
        .ok_or_else(|| format!("{name} missing from captured environment").into())
}

fn executable(path: &Path, content: &str) -> TestResult {
    fs::write(path, content)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}
