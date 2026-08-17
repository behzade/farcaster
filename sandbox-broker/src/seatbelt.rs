//! macOS Seatbelt policy generation.
//!
//! Derived from `OpenAI Codex` `codex-rs/sandboxing/src/seatbelt.rs` at
//! 65ae4c26e088913176a50d6daeb742d00942caee. Pi replaced Codex policy types
//! and network integration with its own narrow, network-blocked policy.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

use regex_lite::escape;

use crate::protocol::{
    Access, DeniedAccess, DenyScope, FilesystemDeny, FilesystemRight, MissingPathBehavior,
    PathScope, SandboxPolicy,
};

pub const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
const BASE_POLICY: &str = include_str!("seatbelt_base_policy.sbpl");
const PROTECTED_METADATA_NAMES: [&str; 2] = [".git", ".pi"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedRight {
    pub access: Access,
    pub path: PathBuf,
    pub scope: PathScope,
    pub approved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedDeny {
    pub access: DeniedAccess,
    pub pattern: String,
    pub scope: DenyScope,
    pub path: Option<PathBuf>,
    pub exempt_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct HardPolicy {
    pub denies: Vec<NormalizedDeny>,
}

impl HardPolicy {
    /// Builds the fixed policy from the broker's own host environment.
    ///
    /// # Errors
    ///
    /// Returns an error if the host home or broker path cannot be made absolute.
    pub fn from_host() -> Result<Self, String> {
        let home = std::env::var_os("HOME").ok_or("broker HOME is missing")?;
        let home = normalize_existing(Path::new(&home))?;
        let broker = std::env::current_exe()
            .map_err(|error| format!("cannot locate broker executable: {error}"))?;
        let broker = normalize_existing(&broker)?;
        let development_cache = std::env::var_os("PI_SANDBOX_DEVELOPMENT_CACHE_ROOT")
            .map(|path| normalize_development_cache_root(Path::new(&path), &home))
            .transpose()?;
        Self::from_paths(&home, &broker, development_cache.as_deref())
    }

    fn from_paths(
        home: &Path,
        broker: &Path,
        development_cache: Option<&Path>,
    ) -> Result<Self, String> {
        let mut policy = Self::base_for_paths(home, broker, development_cache);
        #[cfg(target_os = "macos")]
        for helper in crate::conceal::helper_paths()? {
            push_path_denies(
                &mut policy.denies,
                DeniedAccess::Write,
                &helper,
                DenyScope::File,
            );
        }
        Ok(policy)
    }

    fn base_for_paths(home: &Path, broker: &Path, development_cache: Option<&Path>) -> Self {
        let mut denies = Vec::new();
        for (access, path, scope) in [
            (DeniedAccess::ReadWrite, home.join(".ssh"), DenyScope::Tree),
            (DeniedAccess::ReadWrite, home.join(".aws"), DenyScope::Tree),
            (
                DeniedAccess::ReadWrite,
                home.join(".gnupg"),
                DenyScope::Tree,
            ),
            (
                DeniedAccess::ReadWrite,
                home.join(".pi/agent/auth.json"),
                DenyScope::File,
            ),
            (
                DeniedAccess::ReadWrite,
                home.join(".codex/auth.json"),
                DenyScope::File,
            ),
            (
                DeniedAccess::Read,
                home.join(".pi/agent/extensions/sandbox.json"),
                DenyScope::File,
            ),
            (DeniedAccess::Write, home.join(".pi"), DenyScope::Tree),
            (DeniedAccess::Write, home.join(".codex"), DenyScope::Tree),
            (
                DeniedAccess::ReadWrite,
                broker.to_path_buf(),
                DenyScope::File,
            ),
        ] {
            push_path_denies(&mut denies, access, &path, scope);
        }
        let cache_exemptions: Vec<PathBuf> = development_cache
            .into_iter()
            .map(Path::to_path_buf)
            .collect();
        for pattern in ["/**/*.env", "/**/.env.*"] {
            denies.push(glob_deny_with_exemptions(
                DeniedAccess::ReadWrite,
                pattern,
                cache_exemptions.clone(),
            ));
        }
        denies.push(glob_deny(
            DeniedAccess::Read,
            &format!("{}/**/*.key", home.display()),
        ));
        denies.push(glob_deny(DeniedAccess::Write, "/**/*.key"));
        denies.push(glob_deny(DeniedAccess::Write, "/**/*.pem"));
        Self { denies }
    }
}

fn push_path_denies(
    denies: &mut Vec<NormalizedDeny>,
    access: DeniedAccess,
    path: &Path,
    scope: DenyScope,
) {
    let mut paths = BTreeSet::from([path.to_path_buf()]);
    if let Ok(canonical) = path.canonicalize() {
        paths.insert(canonical);
    }
    denies.extend(paths.into_iter().map(|path| NormalizedDeny {
        access,
        pattern: path.to_string_lossy().into_owned(),
        scope,
        path: Some(path),
        exempt_roots: Vec::new(),
    }));
}

fn glob_deny(access: DeniedAccess, pattern: &str) -> NormalizedDeny {
    glob_deny_with_exemptions(access, pattern, Vec::new())
}

fn glob_deny_with_exemptions(
    access: DeniedAccess,
    pattern: &str,
    exempt_roots: Vec<PathBuf>,
) -> NormalizedDeny {
    NormalizedDeny {
        access,
        pattern: pattern.to_owned(),
        scope: DenyScope::Glob,
        path: None,
        exempt_roots,
    }
}

/// Normalizes all request paths and merges host hard denies.
///
/// # Errors
///
/// Returns an error for relative, missing, mismatched, unsafe, or malformed paths.
pub fn normalize_policy(
    policy: &SandboxPolicy,
    hard: &HardPolicy,
) -> Result<(Vec<NormalizedRight>, Vec<NormalizedDeny>), String> {
    let mut denies = hard.denies.clone();
    for deny in &policy.denies {
        denies.push(normalize_deny(deny)?);
    }

    let mut rights = Vec::new();
    for right in &policy.base_rights {
        rights.push(normalize_right(right, false)?);
    }
    for right in &policy.grants {
        let right = normalize_right(right, true)?;
        if right.path.starts_with("/dev") {
            return Err(format!(
                "approved rights cannot target device paths: {}",
                right.path.display()
            ));
        }
        if denies.iter().any(|deny| deny_matches_right(deny, &right)) {
            return Err(format!(
                "approved right conflicts with a deny: {}",
                right.path.display()
            ));
        }
        rights.push(right);
    }
    if rights.len() > 128 || denies.len() > 128 {
        return Err("filesystem policy has too many entries".to_owned());
    }
    Ok((rights, denies))
}

fn normalize_right(right: &FilesystemRight, approved: bool) -> Result<NormalizedRight, String> {
    if right.access == Access::Read && right.missing_path != MissingPathBehavior::Reject {
        return Err("read rights cannot create a missing path".to_owned());
    }
    match (right.scope, right.missing_path) {
        (PathScope::File, MissingPathBehavior::CreateTree)
        | (PathScope::Tree, MissingPathBehavior::CreateFile) => {
            return Err("missing path behavior does not match right scope".to_owned());
        }
        _ => {}
    }
    let requested_path = Path::new(&right.path);
    let path = normalize_path(requested_path, right.missing_path)?;
    if approved && path != requested_path {
        return Err(format!(
            "approved right changed during broker normalization: {} -> {}",
            requested_path.display(),
            path.display()
        ));
    }
    if path.exists() {
        let metadata = std::fs::metadata(&path)
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
        if right.scope == PathScope::Tree && !metadata.is_dir() {
            return Err(format!("tree right is not a directory: {}", path.display()));
        }
        if right.scope == PathScope::File && metadata.is_dir() {
            return Err(format!("file right is a directory: {}", path.display()));
        }
    }
    Ok(NormalizedRight {
        access: right.access,
        path,
        scope: right.scope,
        approved,
    })
}

fn normalize_deny(deny: &FilesystemDeny) -> Result<NormalizedDeny, String> {
    if deny.pattern.contains('\0') {
        return Err("deny pattern contains NUL".to_owned());
    }
    if deny.scope == DenyScope::Glob {
        assert_absolute_clean(Path::new(&deny.pattern))?;
        seatbelt_regex_for_glob(&deny.pattern)?;
        return Ok(NormalizedDeny {
            access: deny.access,
            pattern: deny.pattern.clone(),
            scope: deny.scope,
            path: None,
            exempt_roots: Vec::new(),
        });
    }
    let path = normalize_path(Path::new(&deny.pattern), MissingPathBehavior::CreateTree)?;
    Ok(NormalizedDeny {
        access: deny.access,
        pattern: path.to_string_lossy().into_owned(),
        scope: deny.scope,
        path: Some(path),
        exempt_roots: Vec::new(),
    })
}

fn normalize_path(path: &Path, missing: MissingPathBehavior) -> Result<PathBuf, String> {
    assert_absolute_clean(path)?;
    if path.exists() {
        return normalize_existing(path);
    }
    if missing == MissingPathBehavior::Reject {
        return Err(format!("path does not exist: {}", path.display()));
    }
    let mut ancestor = path;
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        let name = ancestor
            .file_name()
            .ok_or_else(|| format!("cannot resolve missing path: {}", path.display()))?;
        suffix.push(name.to_owned());
        ancestor = ancestor
            .parent()
            .ok_or_else(|| format!("cannot resolve missing path: {}", path.display()))?;
    }
    let mut normalized = normalize_existing(ancestor)?;
    for part in suffix.into_iter().rev() {
        normalized.push(part);
    }
    Ok(normalized)
}

fn normalize_existing(path: &Path) -> Result<PathBuf, String> {
    assert_absolute_clean(path)?;
    path.canonicalize()
        .map_err(|error| format!("cannot canonicalize {}: {error}", path.display()))
}

fn normalize_development_cache_root(path: &Path, home: &Path) -> Result<PathBuf, String> {
    let path = normalize_existing(path)?;
    if path == home || !path.starts_with(home) {
        return Err(format!(
            "development cache root must be beneath broker HOME: {}",
            path.display()
        ));
    }
    Ok(path)
}

fn assert_absolute_clean(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(format!("path must be absolute: {}", path.display()));
    }
    if path.as_os_str().as_encoded_bytes().contains(&0) {
        return Err("path contains NUL".to_owned());
    }
    if path
        .components()
        .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(format!("path must not contain . or ..: {}", path.display()));
    }
    Ok(())
}

fn deny_matches_right(deny: &NormalizedDeny, right: &NormalizedRight) -> bool {
    if deny
        .exempt_roots
        .iter()
        .any(|root| right.path.starts_with(root))
    {
        return false;
    }
    if !deny_applies_to(deny.access, right.access) {
        return false;
    }
    match deny.scope {
        DenyScope::File => deny.path.as_ref().is_some_and(|path| path == &right.path),
        // A broader approved tree may contain a denied subtree. The generated
        // allow rule carves that subtree out and the explicit deny still wins.
        // Reject only rights that target the denied tree itself or a child.
        DenyScope::Tree => deny
            .path
            .as_ref()
            .is_some_and(|path| right.path.starts_with(path)),
        DenyScope::Glob => seatbelt_regex_for_glob(&deny.pattern)
            .ok()
            .and_then(|pattern| regex_lite::Regex::new(&pattern).ok())
            .is_some_and(|regex| regex.is_match(&right.path.to_string_lossy())),
    }
}

fn deny_applies_to(denied: DeniedAccess, requested: Access) -> bool {
    matches!(denied, DeniedAccess::ReadWrite)
        || matches!((denied, requested), (DeniedAccess::Read, Access::Read))
        || matches!((denied, requested), (DeniedAccess::Write, Access::Write))
}

/// Builds arguments for the fixed `/usr/bin/sandbox-exec` binary.
///
/// # Errors
///
/// Returns an error if a deny glob cannot be translated safely.
pub fn build_args(
    command: &[String],
    cwd: &Path,
    rights: &[NormalizedRight],
    denies: &[NormalizedDeny],
    unix_socket_roots: &[PathBuf],
) -> Result<Vec<String>, String> {
    build_args_with_network(
        command,
        cwd,
        rights,
        denies,
        unix_socket_roots,
        &crate::validation::ValidatedNetworkPolicy::Blocked,
    )
}

/// Builds Seatbelt arguments with blocked or proxy-only network access.
///
/// # Errors
///
/// Returns an error if the command is empty or a file, socket, deny, or proxy
/// rule cannot be translated safely.
pub fn build_args_with_network(
    command: &[String],
    cwd: &Path,
    rights: &[NormalizedRight],
    denies: &[NormalizedDeny],
    unix_socket_roots: &[PathBuf],
    network: &crate::validation::ValidatedNetworkPolicy,
) -> Result<Vec<String>, String> {
    if command.is_empty() {
        return Err("command is empty".to_owned());
    }
    let mut params = Vec::new();
    let read_roots = rights
        .iter()
        .filter(|right| matches!(right.access, Access::Read | Access::Write))
        .cloned()
        .collect::<Vec<_>>();
    let write_roots = rights
        .iter()
        .filter(|right| right.access == Access::Write)
        .cloned()
        .collect::<Vec<_>>();
    let read_policy = build_access_policy(
        "file-read*",
        "READABLE_ROOT",
        cwd,
        &read_roots,
        denies,
        Access::Read,
        &mut params,
    );
    let write_policy = build_access_policy(
        "file-write*",
        "WRITABLE_ROOT",
        cwd,
        &write_roots,
        denies,
        Access::Write,
        &mut params,
    );
    let deny_policy = build_explicit_deny_policy(denies)?;
    let socket_policy = build_unix_socket_policy(unix_socket_roots, &mut params);
    let network_policy = match network {
        crate::validation::ValidatedNetworkPolicy::Blocked => String::new(),
        crate::validation::ValidatedNetworkPolicy::Loopback => {
            build_local_network_policy(cwd, &write_roots, denies, &mut params)?
        }
        crate::validation::ValidatedNetworkPolicy::Proxy {
            tcp_port,
            allow_local_binding,
            ..
        } => {
            let proxy = format!("(allow network-outbound (remote ip \"localhost:{tcp_port}\"))");
            if *allow_local_binding {
                format!(
                    "{}\n{proxy}",
                    build_local_network_policy(cwd, &write_roots, denies, &mut params)?
                )
            } else {
                proxy
            }
        }
    };
    let policy = [
        BASE_POLICY,
        &read_policy,
        &write_policy,
        &deny_policy,
        &socket_policy,
        &network_policy,
    ]
    .join("\n");

    let mut args = vec!["-p".to_owned(), policy];
    args.extend(
        params
            .into_iter()
            .map(|(key, path)| format!("-D{key}={}", path.to_string_lossy())),
    );
    args.push("--".to_owned());
    args.extend_from_slice(command);
    Ok(args)
}

fn build_local_network_policy(
    cwd: &Path,
    write_roots: &[NormalizedRight],
    denies: &[NormalizedDeny],
    params: &mut Vec<(String, PathBuf)>,
) -> Result<String, String> {
    let mut lines = vec![
        "(allow network-bind (local ip \"localhost:*\"))".to_owned(),
        "(allow network-inbound (local ip \"localhost:*\"))".to_owned(),
        "(allow network-outbound (remote ip \"localhost:*\"))".to_owned(),
    ];
    let writable_directories = write_roots
        .iter()
        .enumerate()
        .filter(|(_, root)| root.scope == PathScope::Tree)
        .collect::<Vec<_>>();
    if !writable_directories.is_empty() {
        lines.push("(allow system-socket (socket-domain AF_UNIX))".to_owned());
    }
    for (index, root) in writable_directories {
        let key = format!("LOCAL_UNIX_SOCKET_ROOT_{index}");
        params.push((key.clone(), root.path.clone()));
        let path_filter = build_network_root_requirement(root, &key, cwd, denies, params)?;
        lines.push(format!(
            "(allow network-bind (local unix-socket {path_filter}))"
        ));
    }
    Ok(lines.join("\n"))
}

fn build_network_root_requirement(
    root: &NormalizedRight,
    root_key: &str,
    cwd: &Path,
    denies: &[NormalizedDeny],
    params: &mut Vec<(String, PathBuf)>,
) -> Result<String, String> {
    let mut requirements = vec![build_root_requirement(
        root,
        root_key,
        cwd,
        denies,
        Access::Write,
        params,
    )];
    for deny in denies {
        if !deny_applies_to(deny.access, Access::Write) || deny.scope != DenyScope::Glob {
            continue;
        }
        requirements.push(format!("(require-not {})", glob_deny_matcher(deny)?));
    }
    Ok(format!("(require-all {})", requirements.join(" ")))
}

fn build_unix_socket_policy(
    socket_roots: &[PathBuf],
    params: &mut Vec<(String, PathBuf)>,
) -> String {
    if socket_roots.is_empty() {
        return String::new();
    }
    let mut policy = String::from("(allow system-socket (socket-domain AF_UNIX))\n");
    for (index, path) in socket_roots.iter().enumerate() {
        let key = format!("UNIX_SOCKET_PATH_{index}");
        params.push((key.clone(), path.clone()));
        writeln!(
            policy,
            "(allow network-bind (local unix-socket (literal (param \"{key}\"))))"
        )
        .expect("writing to a String cannot fail");
        writeln!(
            policy,
            "(allow network-outbound (remote unix-socket (literal (param \"{key}\"))))"
        )
        .expect("writing to a String cannot fail");
    }
    policy
}

fn build_access_policy(
    action: &str,
    prefix: &str,
    cwd: &Path,
    roots: &[NormalizedRight],
    denies: &[NormalizedDeny],
    access: Access,
    params: &mut Vec<(String, PathBuf)>,
) -> String {
    let mut components = Vec::new();
    for (index, root) in roots.iter().enumerate() {
        let root_key = format!("{prefix}_{index}");
        params.push((root_key.clone(), root.path.clone()));
        components.push(build_root_requirement(
            root, &root_key, cwd, denies, access, params,
        ));
    }
    if components.is_empty() {
        String::new()
    } else {
        format!("(allow {action}\n{}\n)", components.join("\n"))
    }
}

fn build_root_requirement(
    root: &NormalizedRight,
    root_key: &str,
    cwd: &Path,
    denies: &[NormalizedDeny],
    access: Access,
    params: &mut Vec<(String, PathBuf)>,
) -> String {
    if root.scope == PathScope::File {
        return format!("(literal (param \"{root_key}\"))");
    }
    let mut requirements = vec![format!(
        "(require-any (literal (param \"{root_key}\")) (subpath (param \"{root_key}\")))"
    )];
    let mut excluded = BTreeSet::new();
    for deny in denies {
        if !deny_applies_to(deny.access, access) || deny.scope == DenyScope::Glob {
            continue;
        }
        let Some(path) = &deny.path else { continue };
        if path.starts_with(&root.path) {
            excluded.insert(path.clone());
        }
    }
    for (excluded_index, path) in excluded.into_iter().enumerate() {
        let key = format!("{root_key}_EXCLUDED_{excluded_index}");
        params.push((key.clone(), path));
        requirements.push(format!("(require-not (literal (param \"{key}\")))"));
        requirements.push(format!("(require-not (subpath (param \"{key}\")))"));
    }
    if access == Access::Write
        && !is_control_grant(root)
        && (cwd.starts_with(&root.path) || root.path.starts_with(cwd))
    {
        for name in PROTECTED_METADATA_NAMES {
            let pattern = protected_name_regex(cwd, name).replace('"', "\\\"");
            requirements.push(format!("(require-not (regex #\"{pattern}\"))"));
        }
    }
    format!("(require-all {})", requirements.join(" "))
}

fn is_control_grant(root: &NormalizedRight) -> bool {
    root.approved
        && root
            .path
            .file_name()
            .is_some_and(|name| name == ".git" || name == ".pi")
}

fn protected_name_regex(root: &Path, name: &str) -> String {
    let mut root = root.to_string_lossy().into_owned();
    while root.len() > 1 && root.ends_with('/') {
        root.pop();
    }
    let root = escape(&root);
    let name = escape(name);
    if root == "/" {
        format!(r"^/(.*/)?{name}(/.*)?$")
    } else {
        format!(r"^{root}/(.*/)?{name}(/.*)?$")
    }
}

fn build_explicit_deny_policy(denies: &[NormalizedDeny]) -> Result<String, String> {
    let mut lines = BTreeSet::new();
    for deny in denies {
        let matchers = match deny.scope {
            DenyScope::File => vec![format!("(literal \"{}\")", escape_sbpl(&deny.pattern))],
            DenyScope::Tree => vec![
                format!("(literal \"{}\")", escape_sbpl(&deny.pattern)),
                format!("(subpath \"{}\")", escape_sbpl(&deny.pattern)),
            ],
            DenyScope::Glob => vec![glob_deny_matcher(deny)?],
        };
        for matcher in matchers {
            if matches!(deny.access, DeniedAccess::Read | DeniedAccess::ReadWrite) {
                lines.insert(format!("(deny file-read* {matcher})"));
            }
            if matches!(deny.access, DeniedAccess::Write | DeniedAccess::ReadWrite) {
                lines.insert(format!("(deny file-write* {matcher})"));
            }
        }
    }
    Ok(lines.into_iter().collect::<Vec<_>>().join("\n"))
}

fn glob_deny_matcher(deny: &NormalizedDeny) -> Result<String, String> {
    let regex = seatbelt_regex_for_glob(&deny.pattern)?.replace('"', "\\\"");
    if deny.exempt_roots.is_empty() {
        return Ok(format!("(regex #\"{regex}\")"));
    }
    let mut requirements = vec![format!("(regex #\"{regex}\")")];
    for root in &deny.exempt_roots {
        let root = escape_sbpl(&root.to_string_lossy());
        requirements.push(format!("(require-not (literal \"{root}\"))"));
        requirements.push(format!("(require-not (subpath \"{root}\"))"));
    }
    Ok(format!("(require-all {})", requirements.join(" ")))
}

fn escape_sbpl(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Verifies that the fixed Seatbelt binary can apply a minimal hard policy.
///
/// # Errors
///
/// Returns an error when policy generation, process start, or sandbox apply fails.
#[cfg(target_os = "macos")]
pub fn self_test(hard: &HardPolicy) -> Result<(), String> {
    let rights = vec![NormalizedRight {
        access: Access::Read,
        path: PathBuf::from("/"),
        scope: PathScope::Tree,
        approved: false,
    }];
    let args = build_args(
        &["/usr/bin/true".to_owned()],
        Path::new("/"),
        &rights,
        &hard.denies,
        &[],
    )?;
    let output = std::process::Command::new(SANDBOX_EXEC)
        .args(args)
        .output()
        .map_err(|error| format!("cannot start Seatbelt self-test: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Seatbelt self-test failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn seatbelt_regex_for_glob(pattern: &str) -> Result<String, String> {
    if pattern.is_empty() || !pattern.starts_with('/') {
        return Err("deny glob must be a non-empty absolute pattern".to_owned());
    }
    let mut regex = String::from("^");
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                if chars.peek() == Some(&'/') {
                    chars.next();
                    regex.push_str("(.*/)?");
                } else {
                    regex.push_str(".*");
                }
            }
            '*' => regex.push_str("[^/]*"),
            '?' => regex.push_str("[^/]"),
            '[' | ']' => {
                return Err("character classes are not supported in v1 deny globs".to_owned());
            }
            _ => regex.push_str(&escape(&ch.to_string())),
        }
    }
    regex.push('$');
    Ok(regex)
}

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(target_os = "macos")]
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("pi-broker-{label}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).expect("create test root");
        path
    }

    #[test]
    fn hard_policy_scopes_key_reads_to_home_but_blocks_all_key_writes() {
        let home = temp_root("hard-policy-home");
        let broker = std::env::current_exe().expect("broker fixture");
        let hard = HardPolicy::base_for_paths(&home, &broker, None);
        assert!(hard.denies.iter().any(|deny| {
            deny.access == DeniedAccess::Read
                && deny.pattern == format!("{}/**/*.key", home.display())
        }));
        assert!(
            hard.denies
                .iter()
                .any(|deny| { deny.access == DeniedAccess::Write && deny.pattern == "/**/*.key" })
        );
        assert!(
            !hard.denies.iter().any(|deny| {
                deny.access == DeniedAccess::ReadWrite && deny.pattern == "/**/*.key"
            })
        );
        fs::remove_dir_all(home).expect("remove hard policy home");
    }

    #[test]
    fn hard_env_denies_exempt_only_the_validated_development_cache() {
        let home = temp_root("cache-exemption-home")
            .canonicalize()
            .expect("canonicalize fake home");
        let cache = home.join(".cache/pi-sandbox");
        fs::create_dir_all(&cache).expect("create cache root");
        let broker = std::env::current_exe().expect("broker fixture");
        let cache = normalize_development_cache_root(&cache, &home).expect("valid cache root");
        let hard = HardPolicy::base_for_paths(&home, &broker, Some(&cache));
        let env_denies = hard
            .denies
            .iter()
            .filter(|deny| deny.pattern == "/**/*.env" || deny.pattern == "/**/.env.*")
            .collect::<Vec<_>>();
        assert_eq!(env_denies.len(), 2);
        assert!(
            env_denies
                .iter()
                .all(|deny| deny.exempt_roots == [cache.clone()])
        );
        let matcher = glob_deny_matcher(env_denies[0]).expect("cache-aware matcher");
        assert!(matcher.contains("require-all"));
        assert!(matcher.contains(&format!("(require-not (subpath \"{}\"))", cache.display())));
        let cached_env = NormalizedRight {
            access: Access::Write,
            path: cache.join("checkout/.env.toml"),
            scope: PathScope::File,
            approved: false,
        };
        let sibling_env = NormalizedRight {
            path: home.join(".cache/pi-sandbox-other/.env.toml"),
            ..cached_env.clone()
        };
        assert!(!deny_matches_right(env_denies[1], &cached_env));
        assert!(deny_matches_right(env_denies[1], &sibling_env));
        assert!(
            hard.denies
                .iter()
                .filter(|deny| deny.pattern.ends_with("*.key") || deny.pattern.ends_with("*.pem"))
                .all(|deny| deny.exempt_roots.is_empty())
        );
        assert!(normalize_development_cache_root(&home, &home).is_err());
        assert!(normalize_development_cache_root(Path::new("/"), &home).is_err());
        fs::remove_dir_all(home).expect("remove hard policy home");
    }

    #[cfg(unix)]
    #[test]
    fn approved_missing_path_replaced_by_symlink_is_rejected() {
        use std::os::unix::fs::symlink;

        let root = temp_root("approved-symlink-race")
            .canonicalize()
            .expect("canonicalize test root");
        let target = root.join("target");
        fs::create_dir(&target).expect("create symlink target");
        let approved_path = root.join("approved-missing");
        assert!(!approved_path.exists());
        symlink(&target, &approved_path).expect("replace approved missing path with symlink");
        let policy = SandboxPolicy {
            base_rights: vec![],
            grants: vec![FilesystemRight {
                access: Access::Write,
                path: approved_path.to_string_lossy().into_owned(),
                scope: PathScope::Tree,
                missing_path: MissingPathBehavior::CreateTree,
            }],
            denies: vec![],
            network: crate::protocol::NetworkPolicy::Blocked,
            unix_socket_roots: vec![],
            output_limit_bytes: 1_024,
        };

        let error = normalize_policy(&policy, &HardPolicy { denies: vec![] })
            .expect_err("broker must reject a changed approved path");
        assert!(error.contains("approved right changed during broker normalization"));
        fs::remove_dir_all(root).expect("remove test root");
    }

    #[test]
    fn approved_device_rights_are_rejected() {
        let device = FilesystemRight {
            access: Access::Write,
            path: "/dev/null".to_owned(),
            scope: PathScope::File,
            missing_path: MissingPathBehavior::Reject,
        };
        let policy = SandboxPolicy {
            base_rights: vec![FilesystemRight {
                access: Access::Read,
                ..device.clone()
            }],
            grants: vec![device],
            denies: vec![],
            network: crate::protocol::NetworkPolicy::Blocked,
            unix_socket_roots: vec![],
            output_limit_bytes: 1_024,
        };
        assert!(
            normalize_policy(&policy, &HardPolicy { denies: vec![] })
                .expect_err("device grant must fail")
                .contains("device paths")
        );
    }

    #[test]
    fn file_and_tree_rights_use_distinct_filters() {
        let rights = vec![
            NormalizedRight {
                access: Access::Read,
                path: PathBuf::from("/input.txt"),
                scope: PathScope::File,
                approved: false,
            },
            NormalizedRight {
                access: Access::Write,
                path: PathBuf::from("/work"),
                scope: PathScope::Tree,
                approved: false,
            },
        ];
        let args = build_args(
            &["/usr/bin/true".to_owned()],
            Path::new("/work"),
            &rights,
            &[],
            &[],
        )
        .expect("policy");
        let policy = &args[1];
        assert!(policy.contains("(literal (param \"READABLE_ROOT_0\"))"));
        assert!(policy.contains("(literal (param \"WRITABLE_ROOT_0\"))"));
        assert!(policy.contains("(subpath (param \"WRITABLE_ROOT_0\"))"));
    }

    #[test]
    fn unix_socket_roots_add_only_exact_path_socket_rules() {
        let socket = PathBuf::from("/nix/var/nix/daemon-socket/socket");
        let args = build_args(
            &["/usr/bin/true".to_owned()],
            Path::new("/work"),
            &[],
            &[],
            std::slice::from_ref(&socket),
        )
        .expect("policy");
        let policy = &args[1];
        assert!(policy.contains("(allow system-socket (socket-domain AF_UNIX))"));
        assert!(policy.contains(
            "(allow network-outbound (remote unix-socket (literal (param \"UNIX_SOCKET_PATH_0\"))))"
        ));
        assert!(!policy.contains("(subpath (param \"UNIX_SOCKET_PATH_0\"))"));
        assert!(!policy.contains("\n(allow network-outbound)\n"));
        assert!(
            args.iter()
                .any(|arg| { arg == "-DUNIX_SOCKET_PATH_0=/nix/var/nix/daemon-socket/socket" })
        );
    }

    #[test]
    fn proxy_network_allows_only_one_loopback_port() {
        let rights = vec![NormalizedRight {
            access: Access::Write,
            path: PathBuf::from("/work"),
            scope: PathScope::Tree,
            approved: false,
        }];
        let args = build_args_with_network(
            &["/usr/bin/true".to_owned()],
            Path::new("/work"),
            &rights,
            &[],
            &[],
            &crate::validation::ValidatedNetworkPolicy::Proxy {
                tcp_port: 43_127,
                unix_socket: PathBuf::from("/tmp/proxy.sock"),
                allow_local_binding: false,
            },
        )
        .expect("policy");
        let policy = &args[1];
        assert!(policy.contains("(allow network-outbound (remote ip \"localhost:43127\"))"));
        assert!(!policy.contains("(remote ip \"localhost:*\")"));
        assert!(!policy.contains("LOCAL_UNIX_SOCKET_ROOT"));
        assert!(!policy.contains("\n(allow network-outbound)\n"));
    }

    #[test]
    fn proxy_with_local_binding_keeps_scoped_unix_bind_rules() {
        let rights = vec![NormalizedRight {
            access: Access::Write,
            path: PathBuf::from("/work"),
            scope: PathScope::Tree,
            approved: false,
        }];
        let args = build_args_with_network(
            &["/usr/bin/true".to_owned()],
            Path::new("/work"),
            &rights,
            &[],
            &[],
            &crate::validation::ValidatedNetworkPolicy::Proxy {
                tcp_port: 43_127,
                unix_socket: PathBuf::from("/tmp/proxy.sock"),
                allow_local_binding: true,
            },
        )
        .expect("policy");
        let policy = &args[1];
        assert!(policy.contains("(remote ip \"localhost:43127\")"));
        assert!(policy.contains("(remote ip \"localhost:*\")"));
        assert!(policy.contains("(allow network-bind (local unix-socket"));
        assert!(!policy.contains("(remote unix-socket"));
        assert!(
            args.iter()
                .any(|arg| arg == "-DLOCAL_UNIX_SOCKET_ROOT_0=/work")
        );
    }

    #[test]
    fn loopback_network_allows_local_servers_without_broad_outbound_access() {
        let rights = vec![
            NormalizedRight {
                access: Access::Write,
                path: PathBuf::from("/work"),
                scope: PathScope::Tree,
                approved: false,
            },
            NormalizedRight {
                access: Access::Write,
                path: PathBuf::from("/work/exact.sock"),
                scope: PathScope::File,
                approved: true,
            },
        ];
        let denies = vec![
            NormalizedDeny {
                access: DeniedAccess::Write,
                pattern: "/work/blocked.sock".to_owned(),
                scope: DenyScope::File,
                path: Some(PathBuf::from("/work/blocked.sock")),
                exempt_roots: Vec::new(),
            },
            NormalizedDeny {
                access: DeniedAccess::Write,
                pattern: "/work/**/*.secret".to_owned(),
                scope: DenyScope::Glob,
                path: None,
                exempt_roots: Vec::new(),
            },
        ];
        let args = build_args_with_network(
            &["/usr/bin/true".to_owned()],
            Path::new("/work"),
            &rights,
            &denies,
            &[],
            &crate::validation::ValidatedNetworkPolicy::Loopback,
        )
        .expect("policy");
        let policy = &args[1];
        assert!(policy.contains("(allow network-bind (local ip \"localhost:*\"))"));
        assert!(policy.contains("(allow network-inbound (local ip \"localhost:*\"))"));
        assert!(policy.contains("(allow network-outbound (remote ip \"localhost:*\"))"));
        assert!(policy.contains("(allow system-socket (socket-domain AF_UNIX))"));
        assert!(policy.contains("(allow network-bind (local unix-socket"));
        assert!(!policy.contains("(network-inbound (local unix-socket"));
        assert!(!policy.contains("(remote unix-socket"));
        assert!(
            args.iter()
                .any(|arg| arg == "-DLOCAL_UNIX_SOCKET_ROOT_0=/work")
        );
        assert!(
            args.iter()
                .any(|arg| { arg == "-DLOCAL_UNIX_SOCKET_ROOT_0_EXCLUDED_0=/work/blocked.sock" })
        );
        assert!(
            !args
                .iter()
                .any(|arg| arg.starts_with("-DLOCAL_UNIX_SOCKET_ROOT_1="))
        );
        let unix_bind = policy
            .lines()
            .find(|line| line.contains("(allow network-bind (local unix-socket"))
            .expect("Unix bind policy");
        assert!(unix_bind.contains("LOCAL_UNIX_SOCKET_ROOT_0_EXCLUDED_0"));
        assert!(unix_bind.contains("^/work/(.*/)?\\.pi(/.*)?$"));
        assert!(unix_bind.contains("\\.secret$"));
        assert!(!policy.contains("(remote ip \"*:*\")"));
        assert!(!policy.contains("\n(allow network-outbound)\n"));
    }

    #[test]
    fn broad_writes_protect_control_names_only_below_the_workspace() {
        let rights = vec![NormalizedRight {
            access: Access::Write,
            path: PathBuf::from("/"),
            scope: PathScope::Tree,
            approved: false,
        }];
        let args = build_args(
            &["/usr/bin/true".to_owned()],
            Path::new("/work"),
            &rights,
            &[],
            &[],
        )
        .expect("policy");
        let policy = &args[1];
        assert!(policy.contains("^/work/(.*/)?\\.git(/.*)?$"));
        assert!(policy.contains("^/work/(.*/)?\\.pi(/.*)?$"));
        assert!(!policy.contains("^/(.*/)?\\.git(/.*)?$"));
    }

    #[test]
    fn cache_roots_do_not_block_package_manager_git_metadata() {
        let rights = vec![NormalizedRight {
            access: Access::Write,
            path: PathBuf::from("/home/user/.cargo/git"),
            scope: PathScope::Tree,
            approved: false,
        }];
        let args = build_args(
            &["/usr/bin/true".to_owned()],
            Path::new("/work"),
            &rights,
            &[],
            &[],
        )
        .expect("policy");
        assert!(!args[1].contains("^/home/user/\\.cargo/git/(.*/)?\\.git"));
        assert!(!args[1].contains("^/home/user/\\.cargo/git/(.*/)?\\.pi"));
    }

    #[test]
    fn cache_root_inside_workspace_keeps_workspace_control_carveouts() {
        let rights = vec![NormalizedRight {
            access: Access::Write,
            path: PathBuf::from("/work/.cache/package-manager"),
            scope: PathScope::Tree,
            approved: false,
        }];
        let args = build_args(
            &["/usr/bin/true".to_owned()],
            Path::new("/work"),
            &rights,
            &[],
            &[],
        )
        .expect("policy");
        assert!(args[1].contains("^/work/(.*/)?\\.git(/.*)?$"));
        assert!(args[1].contains("^/work/(.*/)?\\.pi(/.*)?$"));
    }

    #[test]
    fn exact_control_grant_drops_metadata_carveout() {
        let rights = vec![NormalizedRight {
            access: Access::Write,
            path: PathBuf::from("/work/.git"),
            scope: PathScope::Tree,
            approved: true,
        }];
        let args = build_args(
            &["/usr/bin/true".to_owned()],
            Path::new("/work"),
            &rights,
            &[],
            &[],
        )
        .expect("policy");
        assert!(!args[1].contains("^/work/\\.git/\\.git"));
    }

    #[test]
    fn approved_parent_tree_keeps_denied_children_carved_out() {
        let tree_deny = NormalizedDeny {
            access: DeniedAccess::ReadWrite,
            pattern: "/home/user/.ssh".to_owned(),
            scope: DenyScope::Tree,
            path: Some(PathBuf::from("/home/user/.ssh")),
            exempt_roots: Vec::new(),
        };
        let file_deny = NormalizedDeny {
            access: DeniedAccess::ReadWrite,
            pattern: "/home/user/auth.json".to_owned(),
            scope: DenyScope::File,
            path: Some(PathBuf::from("/home/user/auth.json")),
            exempt_roots: Vec::new(),
        };
        let glob_deny = glob_deny(DeniedAccess::ReadWrite, "/**/*.key");
        let parent = NormalizedRight {
            access: Access::Write,
            path: PathBuf::from("/home/user"),
            scope: PathScope::Tree,
            approved: true,
        };
        let child = NormalizedRight {
            path: PathBuf::from("/home/user/.ssh/config"),
            ..parent.clone()
        };
        assert!(!deny_matches_right(&tree_deny, &parent));
        assert!(!deny_matches_right(&file_deny, &parent));
        assert!(!deny_matches_right(&glob_deny, &parent));
        assert!(deny_matches_right(&tree_deny, &child));

        let args = build_args(
            &["/usr/bin/true".to_owned()],
            Path::new("/work"),
            &[parent],
            &[tree_deny, file_deny, glob_deny],
            &[],
        )
        .expect("policy");
        assert!(args[1].contains("WRITABLE_ROOT_0_EXCLUDED_0"));
        assert!(args[1].contains("WRITABLE_ROOT_0_EXCLUDED_1"));
        assert!(args[1].contains("(deny file-write* (regex"));
    }

    #[test]
    fn tree_denies_cover_the_root_and_descendants() {
        let denies = vec![NormalizedDeny {
            access: DeniedAccess::ReadWrite,
            pattern: "/secret".to_owned(),
            scope: DenyScope::Tree,
            path: Some(PathBuf::from("/secret")),
            exempt_roots: Vec::new(),
        }];
        let policy = build_explicit_deny_policy(&denies).expect("deny policy");
        assert!(policy.contains("(deny file-read* (literal \"/secret\"))"));
        assert!(policy.contains("(deny file-read* (subpath \"/secret\"))"));
        assert!(policy.contains("(deny file-write* (literal \"/secret\"))"));
        assert!(policy.contains("(deny file-write* (subpath \"/secret\"))"));
    }

    #[test]
    fn dotted_deny_globs_are_rejected() {
        let deny = FilesystemDeny {
            access: DeniedAccess::ReadWrite,
            pattern: "/work/dir/../*.secret".to_owned(),
            scope: DenyScope::Glob,
        };
        assert!(normalize_deny(&deny).is_err());
    }

    #[test]
    fn globstar_slash_matches_zero_or_more_folders() {
        let regex = seatbelt_regex_for_glob("/**/*.env").expect("glob");
        let regex = regex_lite::Regex::new(&regex).expect("regex");
        assert!(regex.is_match("/.env"));
        assert!(regex.is_match("/repo/nested/.env"));
        assert!(!regex.is_match("/repo/.environment"));
    }

    #[cfg(unix)]
    #[test]
    fn hard_deny_keeps_the_target_of_a_symlink_alias() {
        use std::os::unix::fs::symlink;

        let root = temp_root("hard-alias");
        let home = root.join("home");
        let target = root.join("secret-target");
        fs::create_dir_all(&home).expect("create fake home");
        fs::create_dir_all(&target).expect("create secret target");
        symlink(&target, home.join(".ssh")).expect("create protected alias");
        let mut denies = Vec::new();
        push_path_denies(
            &mut denies,
            DeniedAccess::ReadWrite,
            &home.join(".ssh"),
            DenyScope::Tree,
        );
        let hard = HardPolicy { denies };
        let policy = SandboxPolicy {
            base_rights: vec![],
            grants: vec![FilesystemRight {
                access: Access::Write,
                path: target.to_string_lossy().into_owned(),
                scope: PathScope::Tree,
                missing_path: MissingPathBehavior::Reject,
            }],
            denies: vec![],
            network: crate::protocol::NetworkPolicy::Blocked,
            unix_socket_roots: vec![],
            output_limit_bytes: 1024,
        };
        assert!(normalize_policy(&policy, &hard).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires an unsandboxed macOS runner"]
    fn real_seatbelt_broad_tree_keeps_denied_child_blocked() {
        let root = temp_root("seatbelt-broad")
            .canonicalize()
            .expect("canonicalize test root");
        let denied_root = root.join("protected");
        fs::create_dir_all(&denied_root).expect("create denied child");
        let allowed_file = root.join("allowed.txt");
        let denied_file = denied_root.join("blocked.txt");
        let rights = vec![NormalizedRight {
            access: Access::Write,
            path: root.clone(),
            scope: PathScope::Tree,
            approved: true,
        }];
        let policy_denies = vec![NormalizedDeny {
            access: DeniedAccess::Write,
            pattern: denied_root.to_string_lossy().into_owned(),
            scope: DenyScope::Tree,
            path: Some(denied_root),
            exempt_roots: Vec::new(),
        }];

        let command = vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            format!(
                "printf allowed > '{}'; printf blocked > '{}'",
                allowed_file.display(),
                denied_file.display()
            ),
        ];
        let args = build_args(&command, &root, &rights, &policy_denies, &[]).expect("policy");
        let output = Command::new(SANDBOX_EXEC)
            .args(args)
            .current_dir(&root)
            .output()
            .expect("run sandbox-exec");
        assert!(!output.status.success());
        assert_eq!(
            fs::read_to_string(&allowed_file).expect("read allowed file"),
            "allowed"
        );
        assert!(!denied_file.exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires an unsandboxed macOS runner"]
    fn real_seatbelt_allows_workspace_write_and_blocks_git() {
        let root = temp_root("seatbelt")
            .canonicalize()
            .expect("canonicalize test root");
        let git = root.join("nested/repository/.git");
        fs::create_dir_all(&git).expect("create nested git control root");
        let cache = temp_root("seatbelt-cache")
            .canonicalize()
            .expect("canonicalize cache root");
        let cache_git = cache.join("checkouts/package/.git");
        fs::create_dir_all(&cache_git).expect("create cache git metadata");
        let allowed = root.join("allowed.txt");
        let cache_allowed = cache_git.join("index.lock");
        let protected = git.join("config");
        let rights = vec![
            NormalizedRight {
                access: Access::Read,
                path: PathBuf::from("/"),
                scope: PathScope::Tree,
                approved: false,
            },
            NormalizedRight {
                access: Access::Write,
                path: root.clone(),
                scope: PathScope::Tree,
                approved: false,
            },
            NormalizedRight {
                access: Access::Write,
                path: cache.clone(),
                scope: PathScope::Tree,
                approved: false,
            },
        ];
        let write_allowed = vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            format!("printf ok > '{}'", allowed.display()),
        ];
        let args = build_args(&write_allowed, &root, &rights, &[], &[]).expect("allowed policy");
        let output = Command::new(SANDBOX_EXEC)
            .args(args)
            .current_dir(&root)
            .output()
            .expect("run sandbox-exec");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "allowed write failed: {stderr}");
        assert_eq!(
            fs::read_to_string(&allowed).expect("read allowed file"),
            "ok"
        );

        let write_cache_git = vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            format!("printf cache > '{}'", cache_allowed.display()),
        ];
        let args = build_args(&write_cache_git, &root, &rights, &[], &[]).expect("cache policy");
        let output = Command::new(SANDBOX_EXEC)
            .args(args)
            .current_dir(&root)
            .output()
            .expect("run sandbox-exec");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "cache write failed: {stderr}");
        assert_eq!(
            fs::read_to_string(&cache_allowed).expect("read cache file"),
            "cache"
        );

        let write_protected = vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            format!("printf bad > '{}'", protected.display()),
        ];
        let args =
            build_args(&write_protected, &root, &rights, &[], &[]).expect("protected policy");
        let output = Command::new(SANDBOX_EXEC)
            .args(args)
            .current_dir(&root)
            .output()
            .expect("run sandbox-exec");
        assert!(!output.status.success());
        assert!(!protected.exists());
    }
}
