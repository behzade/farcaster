//! Whole-agent sandbox composition and nono CLI delivery.

use std::{
    io::Write as _,
    path::{Path, PathBuf},
    process::Command,
};

use super::super::core::policy;
use super::super::{
    AccessPolicy, GrantStore, NetworkConfiguration, SandboxPaths, SandboxRuntime, SandboxedCommand,
    contract::SandboxRuntimeKind,
};
#[cfg(test)]
use super::super::{FilesystemAccess, NetworkAccess};

impl super::super::core::PolicyValidator for SandboxRuntime {
    fn validate(&self, policy: &[u8]) -> Result<(), String> {
        validate_policy_bytes(self, policy)
    }
}

pub(crate) fn prepare_sandboxed_command(
    nono: &SandboxRuntime,
    agent_program: &Path,
    prefix_args: &[String],
    paths: SandboxPaths<'_>,
    access: AccessPolicy,
    grants: Option<&GrantStore>,
    network: &NetworkConfiguration,
) -> Result<SandboxedCommand, String> {
    if access.unrestricted() {
        return Ok(direct_command(agent_program, prefix_args));
    }
    let nono_program = match &nono.kind {
        SandboxRuntimeKind::Fixed(program) => validate_nono_program(program)?,
        SandboxRuntimeKind::Unavailable => return Err(missing_nono_error()),
        #[cfg(test)]
        SandboxRuntimeKind::TestBypass => return Ok(direct_command(agent_program, prefix_args)),
    };
    let mut profile = tempfile::NamedTempFile::new()
        .map_err(|error| format!("create Farcaster sandbox profile: {error}"))?;
    profile
        .write_all(&policy::compile(
            paths.project,
            paths.home,
            paths.agent_state,
            paths.temporary,
            access,
            grants.map(GrantStore::resolve).unwrap_or_default(),
            network,
            paths.metadata_read,
        )?)
        .map_err(|error| format!("write Farcaster sandbox profile: {error}"))?;
    profile
        .flush()
        .map_err(|error| format!("flush Farcaster sandbox profile: {error}"))?;
    validate_profile(nono_program, profile.path())?;

    let mut command = Command::new(nono_program);
    command
        .args(["--silent", "run", "--profile"])
        .arg(profile.path())
        .arg("--")
        .arg(agent_program)
        .args(prefix_args);
    Ok(SandboxedCommand {
        command,
        _profile: Some(profile),
    })
}

fn direct_command(agent_program: &Path, prefix_args: &[String]) -> SandboxedCommand {
    let mut command = Command::new(agent_program);
    command.args(prefix_args);
    SandboxedCommand {
        command,
        _profile: None,
    }
}

fn validate_policy_bytes(nono: &SandboxRuntime, policy: &[u8]) -> Result<(), String> {
    let nono_program = match &nono.kind {
        SandboxRuntimeKind::Fixed(program) => validate_nono_program(program)?,
        SandboxRuntimeKind::Unavailable => return Err(missing_nono_error()),
        #[cfg(test)]
        SandboxRuntimeKind::TestBypass => return Ok(()),
    };
    let mut profile = tempfile::NamedTempFile::new()
        .map_err(|error| format!("create Farcaster sandbox profile: {error}"))?;
    profile
        .write_all(policy)
        .and_then(|()| profile.flush())
        .map_err(|error| format!("write Farcaster sandbox profile: {error}"))?;
    validate_profile(nono_program, profile.path())
}

fn validate_profile(nono_program: &Path, profile: &Path) -> Result<(), String> {
    let output = Command::new(nono_program)
        .args(["profile", "validate"])
        .arg(profile)
        .output()
        .map_err(|error| format!("validate Farcaster sandbox profile: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim().chars().take(8_192).collect::<String>();
    Err(format!(
        "nono rejected the Farcaster sandbox profile ({}): {detail}",
        output.status
    ))
}

fn validate_nono_program(program: &Path) -> Result<&Path, String> {
    if program.is_absolute() && is_executable_file(program) {
        Ok(program)
    } else {
        Err(format!(
            "FARCASTER_NONO_PATH must name a fixed executable: {}",
            program.display()
        ))
    }
}

fn missing_nono_error() -> String {
    "nono executable not found; bundle it next to Farcaster or set FARCASTER_NONO_PATH".to_owned()
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

pub(crate) fn configured_sandbox_runtime(value: Option<std::ffi::OsString>) -> SandboxRuntime {
    resolve_nono_program(
        value,
        std::env::current_exe().ok().as_deref(),
        std::env::var_os("PATH"),
    )
}

fn resolve_nono_program(
    value: Option<std::ffi::OsString>,
    current_executable: Option<&Path>,
    search_path: Option<std::ffi::OsString>,
) -> SandboxRuntime {
    if let Some(program) = value.filter(|value| !value.is_empty()) {
        return SandboxRuntime::fixed(PathBuf::from(program));
    }
    let sibling = current_executable
        .and_then(Path::parent)
        .map(|directory| directory.join("nono"));
    if let Some(program) = sibling.filter(|program| is_executable_file(program)) {
        return SandboxRuntime::fixed(program);
    }
    if let Some(search_path) = search_path {
        for directory in std::env::split_paths(&search_path) {
            let candidate = directory.join("nono");
            if is_executable_file(&candidate)
                && let Ok(program) = candidate.canonicalize()
            {
                return SandboxRuntime::fixed(program);
            }
        }
    }
    SandboxRuntime::unavailable()
}

#[cfg(test)]
pub(crate) const fn test_sandbox_bypass() -> SandboxRuntime {
    SandboxRuntime::test_bypass()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn path_executable(name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let path = std::env::var_os("PATH").ok_or("PATH is unavailable")?;
        std::env::split_paths(&path)
            .map(|directory| directory.join(name))
            .find(|candidate| is_executable_file(candidate))
            .ok_or_else(|| format!("{name} is unavailable on PATH").into())
    }

    #[cfg(unix)]
    fn run_real_nono(
        project: &Path,
        home: &Path,
        temporary: &Path,
        program: &Path,
        arguments: &[String],
    ) -> Result<std::process::Output, Box<dyn std::error::Error>> {
        let nono = configured_sandbox_runtime(std::env::var_os("FARCASTER_NONO_PATH"));
        let agent_state = home.join(".pi/agent");
        let mut prepared = prepare_sandboxed_command(
            &nono,
            program,
            arguments,
            SandboxPaths {
                project,
                home,
                agent_state: &agent_state,
                temporary,
                metadata_read: &[],
            },
            AccessPolicy {
                filesystem: FilesystemAccess::Sandboxed,
                network: NetworkAccess::Full,
            },
            None,
            &NetworkConfiguration::default(),
        )?;
        let path = std::env::var_os("PATH").ok_or("PATH is unavailable")?;
        Ok(prepared
            .command
            .current_dir(project)
            .env_clear()
            .env("HOME", home)
            .env("TMPDIR", temporary)
            .env("PATH", path)
            .output()?)
    }

    #[test]
    fn resolves_bundled_nono_before_path() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let bundled = root.path().join("nono");
        std::fs::write(&bundled, b"#!/bin/sh\nexit 0\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&bundled, std::fs::Permissions::from_mode(0o700))?;
        }
        let executable = root.path().join("farcaster");
        let resolved = resolve_nono_program(
            None,
            Some(&executable),
            Some(std::env::join_paths([Path::new("/missing")])?),
        );
        let SandboxRuntimeKind::Fixed(program) = resolved.kind else {
            return Err("bundled nono was not resolved".into());
        };
        assert_eq!(program, bundled);
        Ok(())
    }

    #[test]
    fn resolves_path_nono_to_a_fixed_path() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let nono = root.path().join("nono");
        std::fs::write(&nono, b"#!/bin/sh\nexit 0\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&nono, std::fs::Permissions::from_mode(0o700))?;
        }
        let resolved = resolve_nono_program(
            None,
            Some(Path::new("/missing/farcaster")),
            Some(std::env::join_paths([root.path()])?),
        );
        let SandboxRuntimeKind::Fixed(program) = resolved.kind else {
            return Err("PATH nono was not resolved".into());
        };
        assert_eq!(program, nono.canonicalize()?);
        Ok(())
    }

    #[test]
    fn wraps_restricted_agent_with_fixed_nono_and_profile() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = tempfile::tempdir()?;
        let project = root.path().join("project");
        let home = root.path().join("home");
        let temporary = root.path().join("tmp");
        std::fs::create_dir(&project)?;
        std::fs::create_dir(&home)?;
        let agent_state = home.join(".pi/agent");
        std::fs::create_dir_all(&agent_state)?;
        std::fs::create_dir(&temporary)?;
        let nono = root.path().join("nono");
        std::fs::write(&nono, b"#!/bin/sh\nexit 0\n")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&nono, std::fs::Permissions::from_mode(0o700))?;
        }
        let prepared = prepare_sandboxed_command(
            &SandboxRuntime::fixed(nono.clone()),
            Path::new("/agent/pi"),
            &["prefix".into()],
            SandboxPaths {
                project: &project,
                home: &home,
                agent_state: &agent_state,
                temporary: &temporary,
                metadata_read: std::slice::from_ref(&home),
            },
            AccessPolicy {
                filesystem: FilesystemAccess::Sandboxed,
                network: NetworkAccess::Sandboxed,
            },
            None,
            &NetworkConfiguration::default(),
        )?;
        assert_eq!(prepared.command.get_program(), nono.as_os_str());
        let arguments = prepared.command.get_args().collect::<Vec<_>>();
        assert_eq!(arguments[0], "--silent");
        assert!(arguments.windows(2).any(|pair| pair == ["--", "/agent/pi"]));
        let profile_index = arguments
            .iter()
            .position(|argument| *argument == "--profile")
            .ok_or("profile argument")?;
        let profile = std::fs::read(arguments[profile_index + 1])?;
        let profile = serde_json::from_slice::<serde_json::Value>(&profile)?;
        assert_eq!(profile["meta"]["name"], "farcaster-agent");
        assert_eq!(
            profile["unsafe_macos_seatbelt_rules"],
            serde_json::json!([format!(
                "(allow file-read-metadata (literal \"{}\"))",
                home.display()
            )])
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn real_nono_enforces_symlink_targets_in_home_workspace()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir()?;
        let home = root.path().join("home");
        let temporary = root.path().join("tmp");
        std::fs::create_dir_all(home.join(".pi/agent"))?;
        std::fs::create_dir_all(home.join(".config/git"))?;
        std::fs::create_dir_all(home.join(".ssh"))?;
        std::fs::create_dir(&temporary)?;

        let configured_target = root.path().join("git-config");
        std::fs::write(&configured_target, "configured-symlink\n")?;
        let configured_link = home.join(".config/git/config");
        symlink(&configured_target, &configured_link)?;

        let marker = "protected-marker-must-not-leak\n";
        let secret = home.join(".ssh/id_rsa");
        std::fs::write(&secret, marker)?;
        let protected_link = home.join("harmless");
        symlink(&secret, &protected_link)?;

        let cat = path_executable("cat")?;
        let configured = run_real_nono(
            &home,
            &home,
            &temporary,
            &cat,
            &[configured_link.to_string_lossy().into_owned()],
        )?;
        assert!(
            configured.status.success(),
            "real nono denied configured symlink: {}",
            String::from_utf8_lossy(&configured.stderr)
        );
        assert_eq!(configured.stdout, b"configured-symlink\n");

        let protected = run_real_nono(
            &home,
            &home,
            &temporary,
            &cat,
            &[protected_link.to_string_lossy().into_owned()],
        )?;
        assert!(!protected.status.success());
        assert!(!String::from_utf8_lossy(&protected.stdout).contains(marker.trim()));
        Ok(())
    }

    #[test]
    fn unrestricted_or_test_launches_do_not_wrap_agent() -> Result<(), String> {
        let direct = |nono: &SandboxRuntime, access| {
            prepare_sandboxed_command(
                nono,
                Path::new("/agent/pi"),
                &[],
                SandboxPaths {
                    project: Path::new("/unused"),
                    home: Path::new("/unused"),
                    agent_state: Path::new("/unused"),
                    temporary: Path::new("/unused"),
                    metadata_read: &[],
                },
                access,
                None,
                &NetworkConfiguration::default(),
            )
        };
        let unrestricted = direct(
            &SandboxRuntime::fixed(PathBuf::from("/missing/nono")),
            AccessPolicy {
                filesystem: FilesystemAccess::Full,
                network: NetworkAccess::Full,
            },
        )?;
        assert_eq!(unrestricted.command.get_program(), "/agent/pi");
        let restricted = AccessPolicy {
            filesystem: FilesystemAccess::Sandboxed,
            network: NetworkAccess::Sandboxed,
        };
        let test_bypass = direct(&SandboxRuntime::test_bypass(), restricted)?;
        assert_eq!(test_bypass.command.get_program(), "/agent/pi");
        assert!(
            direct(
                &SandboxRuntime::fixed(PathBuf::from("/missing/nono")),
                restricted,
            )
            .is_err()
        );
        Ok(())
    }
}
