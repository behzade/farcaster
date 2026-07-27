//! macOS Seatbelt policy generation.
//!
//! Derived from `OpenAI Codex` `codex-rs/sandboxing/src/seatbelt.rs` at
//! 65ae4c26e088913176a50d6daeb742d00942caee. Pi replaced Codex policy types
//! and network integration with its own narrow, network-blocked policy.

use std::collections::BTreeSet;
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
            (DeniedAccess::ReadWrite, broker, DenyScope::File),
        ] {
            push_path_denies(&mut denies, access, &path, scope);
        }
        for pattern in ["/**/*.env", "/**/.env.*", "/**/*.key"] {
            denies.push(glob_deny(DeniedAccess::ReadWrite, pattern));
        }
        denies.push(glob_deny(DeniedAccess::Write, "/**/*.pem"));
        Ok(Self { denies })
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
    }));
}

fn glob_deny(access: DeniedAccess, pattern: &str) -> NormalizedDeny {
    NormalizedDeny {
        access,
        pattern: pattern.to_owned(),
        scope: DenyScope::Glob,
        path: None,
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
    let path = normalize_path(Path::new(&right.path), right.missing_path)?;
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
        });
    }
    let path = normalize_path(Path::new(&deny.pattern), MissingPathBehavior::CreateTree)?;
    Ok(NormalizedDeny {
        access: deny.access,
        pattern: path.to_string_lossy().into_owned(),
        scope: deny.scope,
        path: Some(path),
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
    if !deny_applies_to(deny.access, right.access) {
        return false;
    }
    match deny.scope {
        DenyScope::File => deny.path.as_ref().is_some_and(|path| path == &right.path),
        DenyScope::Tree => deny
            .path
            .as_ref()
            .is_some_and(|path| right.path.starts_with(path) || path.starts_with(&right.path)),
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
    rights: &[NormalizedRight],
    denies: &[NormalizedDeny],
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
        &read_roots,
        denies,
        Access::Read,
        &mut params,
    );
    let write_policy = build_access_policy(
        "file-write*",
        "WRITABLE_ROOT",
        &write_roots,
        denies,
        Access::Write,
        &mut params,
    );
    let deny_policy = build_explicit_deny_policy(denies)?;
    let policy = [BASE_POLICY, &read_policy, &write_policy, &deny_policy].join("\n");

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

fn build_access_policy(
    action: &str,
    prefix: &str,
    roots: &[NormalizedRight],
    denies: &[NormalizedDeny],
    access: Access,
    params: &mut Vec<(String, PathBuf)>,
) -> String {
    let mut components = Vec::new();
    for (index, root) in roots.iter().enumerate() {
        let root_key = format!("{prefix}_{index}");
        params.push((root_key.clone(), root.path.clone()));
        if root.scope == PathScope::File {
            components.push(format!("(literal (param \"{root_key}\"))"));
            continue;
        }
        let mut requirements = vec![format!("(subpath (param \"{root_key}\"))")];
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
            let key = format!("{prefix}_{index}_EXCLUDED_{excluded_index}");
            params.push((key.clone(), path));
            requirements.push(format!("(require-not (literal (param \"{key}\")))"));
            requirements.push(format!("(require-not (subpath (param \"{key}\")))"));
        }
        if access == Access::Write && !is_control_grant(root) {
            for name in PROTECTED_METADATA_NAMES {
                let pattern = protected_name_regex(&root.path, name).replace('"', "\\\"");
                requirements.push(format!("(require-not (regex #\"{pattern}\"))"));
            }
        }
        components.push(format!("(require-all {})", requirements.join(" ")));
    }
    if components.is_empty() {
        String::new()
    } else {
        format!("(allow {action}\n{}\n)", components.join("\n"))
    }
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
            DenyScope::Glob => {
                let regex = seatbelt_regex_for_glob(&deny.pattern)?.replace('"', "\\\"");
                vec![format!("(regex #\"{regex}\")")]
            }
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
    let args = build_args(&["/usr/bin/true".to_owned()], &rights, &hard.denies)?;
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
        let args = build_args(&["/usr/bin/true".to_owned()], &rights, &[]).expect("policy");
        let policy = &args[1];
        assert!(policy.contains("(literal (param \"READABLE_ROOT_0\"))"));
        assert!(policy.contains("(subpath (param \"WRITABLE_ROOT_0\"))"));
    }

    #[test]
    fn broad_writes_protect_git_and_pi_names() {
        let rights = vec![NormalizedRight {
            access: Access::Write,
            path: PathBuf::from("/work"),
            scope: PathScope::Tree,
            approved: false,
        }];
        let args = build_args(&["/usr/bin/true".to_owned()], &rights, &[]).expect("policy");
        let policy = &args[1];
        assert!(policy.contains("^/work/(.*/)?\\.git(/.*)?$"));
        assert!(policy.contains("^/work/(.*/)?\\.pi(/.*)?$"));
    }

    #[test]
    fn exact_control_grant_drops_metadata_carveout() {
        let rights = vec![NormalizedRight {
            access: Access::Write,
            path: PathBuf::from("/work/.git"),
            scope: PathScope::Tree,
            approved: true,
        }];
        let args = build_args(&["/usr/bin/true".to_owned()], &rights, &[]).expect("policy");
        assert!(!args[1].contains("^/work/\\.git/\\.git"));
    }

    #[test]
    fn tree_denies_cover_the_root_and_descendants() {
        let denies = vec![NormalizedDeny {
            access: DeniedAccess::ReadWrite,
            pattern: "/secret".to_owned(),
            scope: DenyScope::Tree,
            path: Some(PathBuf::from("/secret")),
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
            output_limit_bytes: 1024,
        };
        assert!(normalize_policy(&policy, &hard).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires an unsandboxed macOS runner"]
    fn real_seatbelt_allows_workspace_write_and_blocks_git() {
        let root = temp_root("seatbelt");
        let git = root.join("nested/repository/.git");
        fs::create_dir_all(&git).expect("create nested git control root");
        let allowed = root.join("allowed.txt");
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
        ];
        let write_allowed = vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            format!("printf ok > '{}'", allowed.display()),
        ];
        let args = build_args(&write_allowed, &rights, &[]).expect("allowed policy");
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

        let write_protected = vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            format!("printf bad > '{}'", protected.display()),
        ];
        let args = build_args(&write_protected, &rights, &[]).expect("protected policy");
        let output = Command::new(SANDBOX_EXEC)
            .args(args)
            .current_dir(&root)
            .output()
            .expect("run sandbox-exec");
        assert!(!output.status.success());
        assert!(!protected.exists());
    }
}
