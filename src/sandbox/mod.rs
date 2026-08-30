//! Whole-agent sandbox composition and nono CLI delivery.

pub(crate) mod approval;
mod policy;

use std::{
    io::Write as _,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) use policy::{AccessPolicy, FilesystemAccess, NetworkAccess};

#[derive(Clone, Debug)]
pub(crate) enum NonoExecutable {
    Fixed(PathBuf),
    Unavailable,
    #[cfg(test)]
    TestBypass,
}

pub(crate) use approval::GrantStore;

pub(crate) struct PolicyPaths<'a> {
    pub(crate) project: &'a Path,
    pub(crate) home: &'a Path,
    pub(crate) agent_state: &'a Path,
    pub(crate) temporary: &'a Path,
}

pub(crate) struct PreparedCommand {
    pub(crate) command: Command,
    _profile: Option<tempfile::NamedTempFile>,
}

pub(crate) fn prepare_command(
    nono: &NonoExecutable,
    agent_program: &Path,
    prefix_args: &[String],
    paths: PolicyPaths<'_>,
    access: AccessPolicy,
    grants: Option<&GrantStore>,
    network: &crate::network::NetworkConfiguration,
) -> Result<PreparedCommand, String> {
    if access.unrestricted() {
        let mut command = Command::new(agent_program);
        command.args(prefix_args);
        return Ok(PreparedCommand {
            command,
            _profile: None,
        });
    }
    let nono_program = match nono {
        NonoExecutable::Fixed(program) => validate_nono_program(program)?,
        NonoExecutable::Unavailable => return Err(missing_nono_error()),
        #[cfg(test)]
        NonoExecutable::TestBypass => {
            let mut command = Command::new(agent_program);
            command.args(prefix_args);
            return Ok(PreparedCommand {
                command,
                _profile: None,
            });
        }
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
    Ok(PreparedCommand {
        command,
        _profile: Some(profile),
    })
}

pub(crate) fn validate_policy_bytes(nono: &NonoExecutable, policy: &[u8]) -> Result<(), String> {
    let nono_program = match nono {
        NonoExecutable::Fixed(program) => validate_nono_program(program)?,
        NonoExecutable::Unavailable => return Err(missing_nono_error()),
        #[cfg(test)]
        NonoExecutable::TestBypass => return Ok(()),
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
        .args(["policy", "validate"])
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

pub(crate) fn configured_nono_program(value: Option<std::ffi::OsString>) -> NonoExecutable {
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
) -> NonoExecutable {
    if let Some(program) = value.filter(|value| !value.is_empty()) {
        return NonoExecutable::Fixed(PathBuf::from(program));
    }
    let sibling = current_executable
        .and_then(Path::parent)
        .map(|directory| directory.join("nono"));
    if let Some(program) = sibling.filter(|program| is_executable_file(program)) {
        return NonoExecutable::Fixed(program);
    }
    if let Some(search_path) = search_path {
        for directory in std::env::split_paths(&search_path) {
            let candidate = directory.join("nono");
            if is_executable_file(&candidate)
                && let Ok(program) = candidate.canonicalize()
            {
                return NonoExecutable::Fixed(program);
            }
        }
    }
    NonoExecutable::Unavailable
}

#[cfg(test)]
pub(crate) const fn test_nono_bypass() -> NonoExecutable {
    NonoExecutable::TestBypass
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let NonoExecutable::Fixed(program) = resolved else {
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
        let NonoExecutable::Fixed(program) = resolved else {
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
        let prepared = prepare_command(
            &NonoExecutable::Fixed(nono.clone()),
            Path::new("/agent/pi"),
            &["prefix".into()],
            PolicyPaths {
                project: &project,
                home: &home,
                agent_state: &agent_state,
                temporary: &temporary,
            },
            AccessPolicy {
                filesystem: FilesystemAccess::Sandboxed,
                network: NetworkAccess::Sandboxed,
            },
            None,
            &crate::network::NetworkConfiguration::default(),
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
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&profile)?["meta"]["name"],
            "farcaster-agent"
        );
        Ok(())
    }

    #[test]
    fn unrestricted_or_test_launches_do_not_wrap_agent() -> Result<(), String> {
        let direct = |nono: &NonoExecutable, access| {
            prepare_command(
                nono,
                Path::new("/agent/pi"),
                &[],
                PolicyPaths {
                    project: Path::new("/unused"),
                    home: Path::new("/unused"),
                    agent_state: Path::new("/unused"),
                    temporary: Path::new("/unused"),
                },
                access,
                None,
                &crate::network::NetworkConfiguration::default(),
            )
        };
        let unrestricted = direct(
            &NonoExecutable::Fixed(PathBuf::from("/missing/nono")),
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
        let test_bypass = direct(&NonoExecutable::TestBypass, restricted)?;
        assert_eq!(test_bypass.command.get_program(), "/agent/pi");
        assert!(
            direct(
                &NonoExecutable::Fixed(PathBuf::from("/missing/nono")),
                restricted,
            )
            .is_err()
        );
        Ok(())
    }
}
