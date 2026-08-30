//! Farcaster-owned policy compilation for a complete agent process.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{AccessPolicy, FilesystemAccess, NetworkAccess};

#[derive(Debug, Serialize)]
struct Profile {
    #[serde(rename = "$schema")]
    schema: &'static str,
    extends: &'static str,
    #[serde(skip_serializing_if = "is_false")]
    allow_parent_of_protected: bool,
    meta: ProfileMeta,
    filesystem: FilesystemPolicy,
    network: NetworkPolicy,
}

#[derive(Debug, Serialize)]
struct ProfileMeta {
    name: &'static str,
    version: &'static str,
    description: &'static str,
}

#[derive(Debug, Serialize)]
struct FilesystemPolicy {
    read: Vec<PathBuf>,
    allow: Vec<PathBuf>,
    allow_file: Vec<PathBuf>,
    bypass_protection: Vec<PathBuf>,
}

#[derive(Debug, Serialize)]
struct NetworkPolicy {
    block: bool,
    allow_domain: Vec<String>,
    open_port: Vec<u16>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

pub(crate) fn compile(
    project: &Path,
    home: &Path,
    agent_state: &Path,
    temporary: &Path,
    access: AccessPolicy,
    grants: super::approval::ResolvedGrants,
    network: &crate::network::NetworkConfiguration,
) -> Result<Vec<u8>, String> {
    let project = canonical_directory(project, "project")?;
    let home = canonical_directory(home, "home")?;
    let agent_state = canonical_directory(agent_state, "agent state")?;
    let temporary = canonical_directory(temporary, "temporary directory")?;
    let mut readable = user_runtime_libraries(&home);
    readable.extend(user_configuration(&home));
    let mut writable = vec![temporary.clone(), agent_state.clone()];
    if let Ok(slash_tmp) = Path::new("/tmp").canonicalize()
        && slash_tmp.is_dir()
    {
        writable.push(slash_tmp);
    }
    let mut writable_files = Vec::new();
    if matches!(access.filesystem, FilesystemAccess::Sandboxed) {
        writable.push(project.clone());
        for path in development_storage(&home) {
            if path.is_dir() {
                writable.push(path);
            } else {
                writable_files.push(path);
            }
        }
    } else if matches!(access.filesystem, FilesystemAccess::Full) {
        writable = vec![PathBuf::from("/")];
        writable_files.clear();
    } else {
        readable.push(project);
    }
    if !matches!(access.filesystem, FilesystemAccess::Full) {
        readable.extend(grants.readable);
        if !matches!(access.filesystem, FilesystemAccess::ReadOnly) {
            writable.extend(grants.writable);
            writable_files.extend(grants.writable_files);
        }
    }
    readable.sort();
    readable.dedup();
    writable.sort();
    writable.dedup();
    writable_files.sort();
    writable_files.dedup();

    let unrestricted_network = matches!(access.network, NetworkAccess::Full);
    let mut allowed_network_hosts = crate::network::allowed_network_hosts()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    allowed_network_hosts.extend(network.proxy_hosts.iter().cloned());
    allowed_network_hosts.extend(grants.network_hosts);
    allowed_network_hosts.sort();
    allowed_network_hosts.dedup();
    let mut loopback_ports = crate::network::loopback_ports().to_vec();
    loopback_ports.extend(network.proxy_loopback_ports.iter().copied());
    loopback_ports.extend(grants.loopback_ports);
    loopback_ports.sort_unstable();
    loopback_ports.dedup();
    let profile = Profile {
        schema: "https://nono.sh/schemas/nono-profile.schema.json",
        extends: "default",
        allow_parent_of_protected: matches!(access.filesystem, FilesystemAccess::Full),
        meta: ProfileMeta {
            name: "farcaster-agent",
            version: "1",
            description: "Farcaster policy for a complete coding-agent process",
        },
        filesystem: FilesystemPolicy {
            read: readable,
            allow: writable,
            allow_file: writable_files,
            // The agent host must read and update its own settings, auth, and sessions.
            // Delegated tools share this outer boundary by design.
            bypass_protection: vec![agent_state],
        },
        network: NetworkPolicy {
            block: !unrestricted_network && allowed_network_hosts.is_empty(),
            allow_domain: if unrestricted_network {
                Vec::new()
            } else {
                allowed_network_hosts
            },
            open_port: if unrestricted_network {
                Vec::new()
            } else {
                loopback_ports
            },
        },
    };
    let mut encoded = serde_json::to_vec_pretty(&profile)
        .map_err(|error| format!("encode Farcaster sandbox profile: {error}"))?;
    encoded.push(b'\n');
    Ok(encoded)
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("resolve sandbox {label} {}: {error}", path.display()))?;
    if !path.is_dir() {
        return Err(format!(
            "sandbox {label} is not a directory: {}",
            path.display()
        ));
    }
    Ok(path)
}

fn user_runtime_libraries(home: &Path) -> Vec<PathBuf> {
    [".local/lib"]
        .into_iter()
        .filter_map(|relative| symlink_free_existing_path(home, relative))
        .collect()
}

fn user_configuration(home: &Path) -> Vec<PathBuf> {
    [".gitconfig", ".config/git/config", ".config/git/ignore"]
        .into_iter()
        .filter_map(|relative| symlink_free_path(home, relative))
        .collect()
}

fn development_storage(home: &Path) -> Vec<PathBuf> {
    const PATHS: &[&str] = &[
        ".cache",
        ".local/state",
        ".config/jj",
        ".bun/install/cache",
        ".cargo/.package-cache",
        ".cargo/.package-cache-mutate",
        ".cargo/git",
        ".cargo/registry",
        ".gradle/caches",
        ".gradle/wrapper/dists",
        ".ivy2/cache",
        ".local/share/pnpm/store",
        ".m2/repository",
        ".npm",
        ".nuget/packages",
        ".rustup/downloads",
        ".rustup/tmp",
        ".yarn/berry/cache",
        ".yarn/berry/index",
        ".yarn/berry/metadata",
        ".yarn/berry/mirror",
        ".yarn/berry/virtual",
        "go/pkg/mod",
        "Library/Caches",
        "Library/Logs",
        "Library/pnpm/store",
    ];
    PATHS
        .iter()
        .filter_map(|relative| symlink_free_existing_path(home, relative))
        .collect()
}

fn symlink_free_existing_path(home: &Path, relative: &str) -> Option<PathBuf> {
    let path = symlink_free_path(home, relative)?;
    path.symlink_metadata().ok()?;
    Some(path)
}

fn symlink_free_path(home: &Path, relative: &str) -> Option<PathBuf> {
    let mut path = home.to_owned();
    let mut components = relative.split('/');
    while let Some(component) = components.next() {
        path.push(component);
        match path.symlink_metadata() {
            Ok(metadata) if metadata.file_type().is_symlink() => return None,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                path.extend(components);
                return Some(path);
            }
            Err(_) => return None,
        }
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(
        root: &Path,
        access: AccessPolicy,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let project = root.join("project");
        let home = root.join("home");
        let temporary = root.join("tmp");
        std::fs::create_dir_all(&project)?;
        std::fs::create_dir_all(home.join(".pi/agent"))?;
        std::fs::create_dir_all(home.join(".cargo/registry"))?;
        std::fs::create_dir_all(home.join(".config/git"))?;
        std::fs::create_dir_all(home.join(".local/lib"))?;
        std::fs::write(home.join(".config/git/config"), "[credential]\n")?;
        std::fs::create_dir_all(&temporary)?;
        Ok(serde_json::from_slice(&compile(
            &project,
            &home,
            &home.join(".pi/agent"),
            &temporary,
            access,
            crate::sandbox::approval::ResolvedGrants::default(),
            &crate::network::NetworkConfiguration::default(),
        )?)?)
    }

    #[test]
    fn sandboxed_policy_preserves_agent_state_git_configuration_and_development_storage()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let profile = value(
            root.path(),
            AccessPolicy {
                filesystem: FilesystemAccess::Sandboxed,
                network: NetworkAccess::Sandboxed,
            },
        )?;
        let allow = profile["filesystem"]["allow"]
            .as_array()
            .ok_or("allow must be an array")?;
        assert!(
            allow
                .iter()
                .any(|path| path.as_str().is_some_and(|path| path.ends_with("/project")))
        );
        assert!(allow.iter().any(|path| {
            path.as_str()
                .is_some_and(|path| path.ends_with("/.pi/agent"))
        }));
        assert!(allow.iter().any(|path| {
            path.as_str()
                .is_some_and(|path| path.ends_with("/.cargo/registry"))
        }));
        let read = profile["filesystem"]["read"]
            .as_array()
            .ok_or("read must be an array")?;
        assert!(read.iter().any(|path| {
            path.as_str()
                .is_some_and(|path| path.ends_with("/.local/lib"))
        }));
        assert!(read.iter().any(|path| {
            path.as_str()
                .is_some_and(|path| path.ends_with("/.gitconfig"))
        }));
        assert!(read.iter().any(|path| {
            path.as_str()
                .is_some_and(|path| path.ends_with("/.config/git/config"))
        }));
        assert!(read.iter().any(|path| {
            path.as_str()
                .is_some_and(|path| path.ends_with("/.config/git/ignore"))
        }));
        assert!(
            !profile["filesystem"]["read"]
                .as_array()
                .is_some_and(|paths| paths.iter().any(|path| path == "/"))
        );
        assert!(
            profile["network"]["allow_domain"]
                .as_array()
                .is_some_and(|hosts| hosts.iter().any(|host| host == "127.0.0.1"))
        );
        assert_eq!(profile["network"]["open_port"][0], 8765);
        Ok(())
    }

    #[test]
    fn exact_write_grants_use_nono_file_capabilities() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let project = root.path().join("project");
        let home = root.path().join("home");
        let temporary = root.path().join("tmp");
        let granted_file = root.path().join("granted.txt");
        std::fs::create_dir(&project)?;
        std::fs::create_dir_all(home.join(".pi/agent"))?;
        std::fs::create_dir(&temporary)?;
        std::fs::write(&granted_file, "fixture")?;
        let profile: serde_json::Value = serde_json::from_slice(&compile(
            &project,
            &home,
            &home.join(".pi/agent"),
            &temporary,
            AccessPolicy {
                filesystem: FilesystemAccess::Sandboxed,
                network: NetworkAccess::Sandboxed,
            },
            crate::sandbox::approval::ResolvedGrants {
                writable_files: vec![granted_file.clone()],
                ..Default::default()
            },
            &crate::network::NetworkConfiguration::default(),
        )?)?;
        assert_eq!(
            profile["filesystem"]["allow_file"],
            serde_json::json!([granted_file])
        );
        Ok(())
    }

    #[test]
    fn sandboxed_policy_includes_proxy_destinations() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let project = root.path().join("project");
        let home = root.path().join("home");
        let temporary = root.path().join("tmp");
        std::fs::create_dir(&project)?;
        std::fs::create_dir_all(home.join(".pi/agent"))?;
        std::fs::create_dir(&temporary)?;
        let network = crate::network::NetworkConfiguration {
            proxy_hosts: vec!["proxy.example".into()],
            proxy_loopback_ports: vec![8080],
            app_proxy: None,
        };
        let profile: serde_json::Value = serde_json::from_slice(&compile(
            &project,
            &home,
            &home.join(".pi/agent"),
            &temporary,
            AccessPolicy {
                filesystem: FilesystemAccess::Sandboxed,
                network: NetworkAccess::Sandboxed,
            },
            crate::sandbox::approval::ResolvedGrants::default(),
            &network,
        )?)?;
        assert!(
            profile["network"]["allow_domain"]
                .as_array()
                .is_some_and(|hosts| hosts.iter().any(|host| host == "proxy.example"))
        );
        assert!(
            profile["network"]["open_port"]
                .as_array()
                .is_some_and(|ports| ports.iter().any(|port| port == 8080))
        );
        Ok(())
    }

    #[test]
    fn read_only_policy_does_not_write_workspace() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let profile = value(
            root.path(),
            AccessPolicy {
                filesystem: FilesystemAccess::ReadOnly,
                network: NetworkAccess::Full,
            },
        )?;
        let allow = profile["filesystem"]["allow"]
            .as_array()
            .ok_or("allow must be an array")?;
        assert!(
            !allow
                .iter()
                .any(|path| path.as_str().is_some_and(|path| path.ends_with("/project")))
        );
        assert!(
            profile["filesystem"]["read"]
                .as_array()
                .is_some_and(|paths| paths
                    .iter()
                    .any(|path| path.as_str().is_some_and(|path| path.ends_with("/project"))))
        );
        assert_eq!(profile["network"]["block"], false);
        assert_eq!(profile["network"]["allow_domain"], serde_json::json!([]));
        Ok(())
    }
}
