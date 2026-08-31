//! Farcaster-owned policy compilation for a complete agent process.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{AccessPolicy, FilesystemAccess, NetworkAccess, NetworkConfiguration};

#[derive(Debug, Serialize)]
struct Profile {
    #[serde(rename = "$schema")]
    schema: &'static str,
    extends: &'static str,
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
    read_file: Vec<PathBuf>,
    allow: Vec<PathBuf>,
    allow_file: Vec<PathBuf>,
    bypass_protection: Vec<PathBuf>,
}

#[derive(Debug, Serialize)]
struct NetworkPolicy {
    block: bool,
    allow_domain: Vec<String>,
    open_port: Vec<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tls_intercept: Option<TlsInterceptPolicy>,
}

#[derive(Debug, Serialize)]
struct TlsInterceptPolicy {
    ca_env_vars: Vec<String>,
}

pub(crate) fn compile(
    project: &Path,
    home: &Path,
    agent_state: &Path,
    temporary: &Path,
    access: AccessPolicy,
    grants: super::approval::ResolvedGrants,
    network: &NetworkConfiguration,
) -> Result<Vec<u8>, String> {
    let project = canonical_directory(project, "project")?;
    let home = canonical_directory(home, "home")?;
    let agent_state = canonical_directory(agent_state, "agent state")?;
    let temporary = canonical_directory(temporary, "temporary directory")?;
    let mut readable = immutable_runtime_roots();
    readable.extend(user_runtime_libraries(&home));
    // Seatbelt preserves the default profile's protected-child denies within this grant.
    // Linux Landlock cannot express deny-within-allow, so it keeps explicit reads below.
    #[cfg(target_os = "macos")]
    readable.push(home.clone());
    let mut readable_files = user_configuration(&home);
    let mut protection_bypasses = grants
        .readable
        .iter()
        .chain(&grants.readable_files)
        .chain(&grants.writable)
        .chain(&grants.writable_files)
        .filter(|path| *path != &project)
        .cloned()
        .collect::<Vec<_>>();
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
        protection_bypasses = vec![PathBuf::from("/")];
    } else {
        readable.push(project.clone());
    }
    if !matches!(access.filesystem, FilesystemAccess::Full) {
        readable.extend(grants.readable);
        readable_files.extend(grants.readable_files);
        if !matches!(access.filesystem, FilesystemAccess::ReadOnly) {
            writable.extend(grants.writable);
            writable_files.extend(grants.writable_files);
        }
    }
    readable.sort();
    readable.dedup();
    readable_files.sort();
    readable_files.dedup();
    writable.sort();
    writable.dedup();
    writable_files.sort();
    writable_files.dedup();
    protection_bypasses.sort();
    protection_bypasses.dedup();

    let unrestricted_network = matches!(access.network, NetworkAccess::Full);
    let mut allowed_network_hosts = super::network::allowed_network_hosts()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    allowed_network_hosts.extend(network.proxy_hosts.iter().cloned());
    allowed_network_hosts.extend(grants.network_hosts);
    allowed_network_hosts.sort();
    allowed_network_hosts.dedup();
    let mut loopback_ports = super::network::loopback_ports().to_vec();
    loopback_ports.extend(network.proxy_loopback_ports.iter().copied());
    loopback_ports.extend(grants.loopback_ports);
    loopback_ports.sort_unstable();
    loopback_ports.dedup();
    let profile = Profile {
        schema: "https://nono.sh/schemas/nono-profile.schema.json",
        extends: "default",
        allow_parent_of_protected: true,
        meta: ProfileMeta {
            name: "farcaster-agent",
            version: "1",
            description: "Farcaster policy for a complete coding-agent process",
        },
        filesystem: FilesystemPolicy {
            read: readable,
            read_file: readable_files,
            allow: writable,
            allow_file: writable_files,
            // Baseline access must not disable protection after symlink resolution.
            // Only explicit user-approved grants and full access bypass protection.
            bypass_protection: protection_bypasses,
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
            tls_intercept: (!network.tls_ca_env_vars.is_empty()).then(|| TlsInterceptPolicy {
                ca_env_vars: network.tls_ca_env_vars.clone(),
            }),
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

fn immutable_runtime_roots() -> Vec<PathBuf> {
    [PathBuf::from("/nix/store")]
        .into_iter()
        .filter(|path| path.is_dir())
        .collect()
}

fn user_runtime_libraries(home: &Path) -> Vec<PathBuf> {
    [".local/lib"]
        .into_iter()
        .filter_map(|relative| existing_user_path(home, relative))
        .collect()
}

fn user_configuration(home: &Path) -> Vec<PathBuf> {
    [".gitconfig", ".config/git/config", ".config/git/ignore"]
        .into_iter()
        .map(|relative| home.join(relative))
        .collect()
}

fn development_storage(home: &Path) -> Vec<PathBuf> {
    const PATHS: &[&str] = &[
        ".cache",
        ".codex",
        ".local/state",
        ".config/jj",
        ".config/opencode",
        ".bun/install/cache",
        ".cargo/.package-cache",
        ".cargo/.package-cache-mutate",
        ".cargo/git",
        ".cargo/registry",
        ".gradle/caches",
        ".gradle/wrapper/dists",
        ".ivy2/cache",
        ".local/share/opencode",
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
        .filter_map(|relative| existing_user_path(home, relative))
        .collect()
}

fn existing_user_path(home: &Path, relative: &str) -> Option<PathBuf> {
    let path = home.join(relative);
    path.metadata().ok()?;
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
        std::fs::create_dir_all(home.join(".codex"))?;
        std::fs::create_dir_all(home.join(".config/git"))?;
        std::fs::create_dir_all(home.join(".config/opencode"))?;
        std::fs::create_dir_all(home.join(".local/lib"))?;
        std::fs::create_dir_all(home.join(".local/share/opencode"))?;
        std::fs::write(home.join(".config/git/config"), "[credential]\n")?;
        std::fs::create_dir_all(&temporary)?;
        Ok(serde_json::from_slice(&compile(
            &project,
            &home,
            &home.join(".pi/agent"),
            &temporary,
            access,
            crate::access::approval::ResolvedGrants::default(),
            &NetworkConfiguration::default(),
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
        assert!(
            allow
                .iter()
                .any(|path| path.as_str().is_some_and(|path| path.ends_with("/.codex")))
        );
        assert!(allow.iter().any(|path| {
            path.as_str()
                .is_some_and(|path| path.ends_with("/.config/opencode"))
        }));
        assert!(allow.iter().any(|path| {
            path.as_str()
                .is_some_and(|path| path.ends_with("/.local/share/opencode"))
        }));
        let read = profile["filesystem"]["read"]
            .as_array()
            .ok_or("read must be an array")?;
        assert!(read.iter().any(|path| {
            path.as_str()
                .is_some_and(|path| path.ends_with("/.local/lib"))
        }));
        #[cfg(target_os = "macos")]
        {
            let home = root.path().join("home").canonicalize()?;
            assert!(read.contains(&serde_json::json!(home)));
        }
        if Path::new("/nix/store").is_dir() {
            assert!(read.iter().any(|path| path == "/nix/store"));
        }
        let read_file = profile["filesystem"]["read_file"]
            .as_array()
            .ok_or("read_file must be an array")?;
        assert!(read_file.iter().any(|path| {
            path.as_str()
                .is_some_and(|path| path.ends_with("/.gitconfig"))
        }));
        assert!(read_file.iter().any(|path| {
            path.as_str()
                .is_some_and(|path| path.ends_with("/.config/git/config"))
        }));
        assert!(read_file.iter().any(|path| {
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
    fn exact_grants_use_nono_scopes_and_bypass_default_protection()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let project = root.path().join("project");
        let home = root.path().join("home");
        let temporary = root.path().join("tmp");
        let readable = root.path().join("readable");
        let readable_file = root.path().join("input.txt");
        let writable_file = root.path().join("output.txt");
        std::fs::create_dir(&project)?;
        std::fs::create_dir_all(home.join(".pi/agent"))?;
        std::fs::create_dir(&temporary)?;
        std::fs::create_dir(&readable)?;
        std::fs::write(&readable_file, "input")?;
        std::fs::write(&writable_file, "output")?;
        let profile: serde_json::Value = serde_json::from_slice(&compile(
            &project,
            &home,
            &home.join(".pi/agent"),
            &temporary,
            AccessPolicy {
                filesystem: FilesystemAccess::Sandboxed,
                network: NetworkAccess::Sandboxed,
            },
            crate::access::approval::ResolvedGrants {
                readable: vec![readable.clone()],
                readable_files: vec![readable_file.clone()],
                writable_files: vec![writable_file.clone()],
                ..Default::default()
            },
            &NetworkConfiguration::default(),
        )?)?;
        assert!(
            profile["filesystem"]["read"]
                .as_array()
                .is_some_and(|paths| paths.contains(&serde_json::json!(readable)))
        );
        assert!(
            profile["filesystem"]["read_file"]
                .as_array()
                .is_some_and(|paths| paths.contains(&serde_json::json!(readable_file)))
        );
        assert_eq!(
            profile["filesystem"]["allow_file"],
            serde_json::json!([writable_file])
        );
        let bypasses = profile["filesystem"]["bypass_protection"]
            .as_array()
            .ok_or("bypass_protection must be an array")?;
        assert!(bypasses.contains(&serde_json::json!(readable)));
        assert!(bypasses.contains(&serde_json::json!(readable_file)));
        assert!(bypasses.contains(&serde_json::json!(writable_file)));
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
        let network = NetworkConfiguration {
            proxy_hosts: vec!["proxy.example".into()],
            proxy_loopback_ports: vec![8080],
            tls_ca_env_vars: vec!["CODEX_CA_CERTIFICATE".into()],
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
            crate::access::approval::ResolvedGrants::default(),
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
        assert_eq!(
            profile["network"]["tls_intercept"]["ca_env_vars"],
            serde_json::json!(["CODEX_CA_CERTIFICATE"])
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
