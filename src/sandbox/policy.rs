//! Farcaster-owned policy compilation for a complete agent process.

use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FilesystemAccess {
    ReadOnly,
    Sandboxed,
    Full,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NetworkAccess {
    Sandboxed,
    Full,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AccessPolicy {
    pub(crate) filesystem: FilesystemAccess,
    pub(crate) network: NetworkAccess,
}

impl AccessPolicy {
    pub(crate) const fn unrestricted(self) -> bool {
        matches!(self.filesystem, FilesystemAccess::Full)
            && matches!(self.network, NetworkAccess::Full)
    }
}

#[derive(Debug, Serialize)]
struct Profile<'a> {
    #[serde(rename = "$schema")]
    schema: &'static str,
    extends: &'static str,
    #[serde(skip_serializing_if = "is_false")]
    allow_parent_of_protected: bool,
    meta: ProfileMeta,
    filesystem: FilesystemPolicy,
    network: NetworkPolicy<'a>,
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
struct NetworkPolicy<'a> {
    block: bool,
    allow_domain: &'a [&'a str],
    open_port: &'a [u16],
}

fn is_false(value: &bool) -> bool {
    !*value
}

const ALLOWED_NETWORK_HOSTS: &[&str] = &[
    "127.0.0.1",
    "::1",
    "localhost",
    "*.amazonaws.com",
    "*.openai.azure.com",
    "api.anthropic.com",
    "api.cohere.ai",
    "api.cohere.com",
    "api.deepseek.com",
    "api.fireworks.ai",
    "api.github.com",
    "api.groq.com",
    "api.mistral.ai",
    "api.openai.com",
    "api.openrouter.ai",
    "api.perplexity.ai",
    "api.together.xyz",
    "api.x.ai",
    "cache.nixos.org",
    "codeload.github.com",
    "crates.io",
    "files.pythonhosted.org",
    "generativelanguage.googleapis.com",
    "aiplatform.googleapis.com",
    "ghcr.io",
    "github.com",
    "index.crates.io",
    "nodejs.org",
    "oauth2.googleapis.com",
    "objects.githubusercontent.com",
    "openrouter.ai",
    "pkg-containers.githubusercontent.com",
    "proxy.golang.org",
    "pypi.org",
    "registry.npmjs.org",
    "release-assets.githubusercontent.com",
    "repo.maven.apache.org",
    "static.crates.io",
    "static.rust-lang.org",
    "storage.googleapis.com",
];
const LOOPBACK_PORTS: &[u16] = &[8765];

pub(crate) fn compile(
    project: &Path,
    home: &Path,
    agent_state: &Path,
    temporary: &Path,
    access: AccessPolicy,
) -> Result<Vec<u8>, String> {
    let project = canonical_directory(project, "project")?;
    let home = canonical_directory(home, "home")?;
    let agent_state = canonical_directory(agent_state, "agent state")?;
    let temporary = canonical_directory(temporary, "temporary directory")?;
    let mut readable = Vec::new();
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
    readable.sort();
    readable.dedup();
    writable.sort();
    writable.dedup();
    writable_files.sort();
    writable_files.dedup();

    let unrestricted_network = matches!(access.network, NetworkAccess::Full);
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
            block: !unrestricted_network && ALLOWED_NETWORK_HOSTS.is_empty(),
            allow_domain: if unrestricted_network {
                &[]
            } else {
                ALLOWED_NETWORK_HOSTS
            },
            open_port: if unrestricted_network {
                &[]
            } else {
                LOOPBACK_PORTS
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
    let mut path = home.to_owned();
    for component in relative.split('/') {
        path.push(component);
        let metadata = path.symlink_metadata().ok()?;
        if metadata.file_type().is_symlink() {
            return None;
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
        std::fs::create_dir_all(&temporary)?;
        Ok(serde_json::from_slice(&compile(
            &project,
            &home,
            &home.join(".pi/agent"),
            &temporary,
            access,
        )?)?)
    }

    #[test]
    fn sandboxed_policy_preserves_agent_state_and_development_storage()
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
            !profile["filesystem"]["read"]
                .as_array()
                .is_some_and(|paths| paths.iter().any(|path| path == "/"))
        );
        assert_eq!(profile["network"]["allow_domain"][0], "127.0.0.1");
        assert_eq!(profile["network"]["open_port"][0], 8765);
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
