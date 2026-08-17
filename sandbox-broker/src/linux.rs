//! Linux Bubblewrap policy and launcher preparation.
//!
//! Adapted from OpenAI Codex commit
//! `65ae4c26e088913176a50d6daeb742d00942caee`, chiefly
//! `codex-rs/linux-sandbox/src/{bwrap.rs,landlock.rs,linux_run_main.rs}`.
//! Pi uses a compile-time fixed Bubblewrap path. Blocked commands receive a
//! small reviewed seccomp filter. Proxied commands re-enter this binary only
//! inside their private network namespace to bridge isolated loopback to one
//! validated host proxy socket.

#![cfg(target_os = "linux")]

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::net::{Ipv4Addr, Shutdown, TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use nix::fcntl::{FcntlArg, FdFlag, fcntl};
use regex_lite::{Regex, escape};

use crate::protocol::{Access, DeniedAccess, DenyScope, PathScope};
use crate::seatbelt::{NormalizedDeny, NormalizedRight};
use crate::validation::ValidatedExec;
use crate::validation::ValidatedNetworkPolicy;

pub const BWRAP: &str = match option_env!("PI_BWRAP_PATH") {
    Some(path) => path,
    None => "/usr/bin/bwrap",
};

const MAX_SCAN_ENTRIES: usize = 200_000;
const MAX_GLOB_MATCHES: usize = 8_192;
const MAX_SCAN_DEPTH: usize = 64;
const PROTECTED_METADATA_NAMES: [&str; 2] = [".git", ".pi"];
pub const PROXY_LOOPBACK_PORT: u16 = 31_128;
const PROXY_LAUNCHER_PATH: &str = "/tmp/.pi-sandbox-network-launcher";

const BPF_LD_W_ABS: u16 = 0x20;
const BPF_JMP_JEQ_K: u16 = 0x15;
#[cfg(target_arch = "x86_64")]
const BPF_JMP_JGE_K: u16 = 0x35;
const BPF_RET_K: u16 = 0x06;
const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
const SECCOMP_RET_ERRNO: u32 = 0x0005_0000;
const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;
const SECCOMP_DATA_ARCH_OFFSET: u32 = 4;
const SECCOMP_DATA_ARGS_OFFSET: u32 = 16;

#[cfg(target_arch = "x86_64")]
const AUDIT_ARCH: u32 = 0xc000_003e;
#[cfg(target_arch = "aarch64")]
const AUDIT_ARCH: u32 = 0xc000_00b7;

/// Owns resources which must remain open until Bubblewrap has started.
pub struct PreparedLaunch {
    pub program: &'static str,
    pub args: Vec<String>,
    pub resources: Vec<File>,
    pub synthetic_directories: Vec<SyntheticDirectory>,
}

/// A missing host directory which Bubblewrap may create as a mount target.
/// It is removed only if it remains an empty directory after the command.
pub struct SyntheticDirectory(PathBuf);

impl Drop for SyntheticDirectory {
    fn drop(&mut self) {
        if fs::read_dir(&self.0).is_ok_and(|mut entries| entries.next().is_none()) {
            let _ = fs::remove_dir(&self.0);
        }
    }
}

/// Builds a fail-closed Bubblewrap invocation for one validated command.
pub fn prepare(request: &ValidatedExec, command: &[String]) -> Result<PreparedLaunch, String> {
    if !request.unix_socket_roots.is_empty() {
        return Err(
            "Unix socket roots are only supported by the macOS Seatbelt backend".to_owned(),
        );
    }
    if !request.rights.iter().any(|right| {
        right.access == Access::Read
            && right.scope == PathScope::Tree
            && right.path == Path::new("/")
    }) {
        return Err("Linux protocol v4 requires an explicit read right for /".to_owned());
    }
    if command.is_empty() {
        return Err("command is empty".to_owned());
    }

    let mut writable = request
        .rights
        .iter()
        .filter(|right| right.access == Access::Write)
        .cloned()
        .collect::<Vec<_>>();
    writable.sort_by_key(|right| path_depth(&right.path));
    reject_missing_concrete_denies(&request.denies, &writable)?;

    let approved_controls = writable
        .iter()
        .filter(|right| right.approved && is_control_root(&right.path))
        .map(|right| right.path.clone())
        .collect::<BTreeSet<_>>();
    let protected = protected_workspace_paths(&request.cwd, &approved_controls)?;
    let synthetic_directories = missing_workspace_control_paths(&request.cwd, &approved_controls);
    let mut denies = concrete_denies(&request.denies, &request.cwd)?;
    denies.sort_by_key(|deny| path_depth(&deny.path));
    reject_writable_symlink_crossings(&denies, &writable)?;
    let needs_hidden_file = denies.iter().any(|deny| {
        !deny.path.is_dir() && matches!(deny.access, DeniedAccess::Read | DeniedAccess::ReadWrite)
    });
    let hidden_file = needs_hidden_file.then(hidden_file_source).transpose()?;
    let (network_enabled, proxy_socket, allow_local_binding) = match &request.network {
        ValidatedNetworkPolicy::Blocked => (false, None, false),
        ValidatedNetworkPolicy::Loopback => (true, None, true),
        ValidatedNetworkPolicy::Proxy {
            unix_socket,
            allow_local_binding,
            ..
        } => (true, Some(unix_socket.clone()), *allow_local_binding),
    };
    let seccomp = (!network_enabled).then(seccomp_file).transpose()?;
    let launcher = network_enabled
        .then(|| {
            let file = File::open(std::env::current_exe().map_err(|error| error.to_string())?)
                .map_err(|error| format!("cannot open network launcher: {error}"))?;
            make_inheritable(&file, "network launcher")?;
            Ok::<File, String>(file)
        })
        .transpose()?;

    // Create only normalized write targets after every read-only policy scan
    // has succeeded, so a rejected request leaves no approved-path artifact.
    create_missing_write_targets(&writable)?;
    let mut args = base_args();
    for right in &writable {
        ensure_existing_type(right)?;
        push_mount(&mut args, "--bind", &right.path, &right.path);
    }
    for path in protected {
        push_mount(&mut args, "--ro-bind", &path, &path);
    }
    for target in &synthetic_directories {
        args.extend([
            "--perms".to_owned(),
            "555".to_owned(),
            "--tmpfs".to_owned(),
            path_string(&target.0),
            "--remount-ro".to_owned(),
            path_string(&target.0),
        ]);
    }
    for deny in denies {
        append_deny(&mut args, &deny, hidden_file.as_ref())?;
    }
    if let Some(socket) = &proxy_socket {
        let directory = socket
            .parent()
            .ok_or_else(|| "network proxy socket has no parent directory".to_owned())?;
        // The workspace can normally write `/tmp`. Mount the unique proxy
        // directory read-only after all write mounts so user code cannot swap
        // the validated socket for a host service path before the bridge opens.
        push_mount(&mut args, "--ro-bind", directory, directory);
    }
    // Install the launcher after parent write mounts so a writable `/tmp`
    // cannot hide or replace this final read-only file mount.
    if let Some(launcher) = &launcher {
        args.extend([
            "--ro-bind-data".to_owned(),
            launcher.as_raw_fd().to_string(),
            PROXY_LAUNCHER_PATH.to_owned(),
        ]);
    }

    if let Some(seccomp) = &seccomp {
        args.push("--seccomp".to_owned());
        args.push(seccomp.as_raw_fd().to_string());
    }
    args.push("--chdir".to_owned());
    args.push(path_string(&request.cwd));
    args.push("--".to_owned());
    if network_enabled {
        args.extend([
            PROXY_LAUNCHER_PATH.to_owned(),
            "__linux_proxy_launch".to_owned(),
            proxy_socket
                .as_ref()
                .map_or_else(|| "-".to_owned(), |socket| path_string(socket)),
            if allow_local_binding {
                "allow-local-binding".to_owned()
            } else {
                "deny-local-binding".to_owned()
            },
            "--".to_owned(),
        ]);
    }
    args.extend_from_slice(command);

    let mut resources = hidden_file.into_iter().collect::<Vec<_>>();
    resources.extend(seccomp);
    resources.extend(launcher);
    Ok(PreparedLaunch {
        program: BWRAP,
        args,
        resources,
        synthetic_directories,
    })
}

fn base_args() -> Vec<String> {
    [
        "--new-session",
        "--die-with-parent",
        "--unshare-user",
        "--unshare-pid",
        "--unshare-net",
        "--unshare-ipc",
        "--unshare-uts",
        "--ro-bind",
        "/",
        "/",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn reject_missing_concrete_denies(
    denies: &[NormalizedDeny],
    writable: &[NormalizedRight],
) -> Result<(), String> {
    for deny in denies {
        if deny.scope == DenyScope::Glob || deny.path.as_ref().is_none_or(|path| path.exists()) {
            continue;
        }
        let path = deny.path.as_ref().expect("checked above");
        if writable
            .iter()
            .any(|right| path.starts_with(&right.path) || right.path.starts_with(path))
        {
            return Err(format!(
                "cannot enforce missing deny below writable root: {}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn create_missing_write_targets(rights: &[NormalizedRight]) -> Result<(), String> {
    for right in rights {
        if right.path.exists() {
            continue;
        }
        let parent = right
            .path
            .parent()
            .ok_or_else(|| format!("missing write path has no parent: {}", right.path.display()))?;
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "cannot create parent of approved target {}: {error}",
                right.path.display()
            )
        })?;
        match right.scope {
            PathScope::Tree => fs::create_dir(&right.path),
            PathScope::File => OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&right.path)
                .map(drop),
        }
        .map_err(|error| {
            format!(
                "cannot create approved target {}: {error}",
                right.path.display()
            )
        })?;
    }
    Ok(())
}

fn ensure_existing_type(right: &NormalizedRight) -> Result<(), String> {
    let metadata = fs::metadata(&right.path).map_err(|error| {
        format!(
            "cannot inspect write root {}: {error}",
            right.path.display()
        )
    })?;
    if right.scope == PathScope::Tree && !metadata.is_dir() {
        return Err(format!(
            "tree write root is not a directory: {}",
            right.path.display()
        ));
    }
    if right.scope == PathScope::File && metadata.is_dir() {
        return Err(format!(
            "file write root is a directory: {}",
            right.path.display()
        ));
    }
    Ok(())
}

fn protected_workspace_paths(
    cwd: &Path,
    approved: &BTreeSet<PathBuf>,
) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    let mut symlink = None;
    let mut visited_entries = 0_usize;
    let mut visited_directories = BTreeSet::new();
    walk(
        cwd,
        0,
        false,
        &mut visited_entries,
        &mut visited_directories,
        &mut |path, file_type| {
            if path
                .file_name()
                .is_some_and(|name| PROTECTED_METADATA_NAMES.iter().any(|item| name == *item))
            {
                if file_type.is_symlink() {
                    symlink = Some(path.to_path_buf());
                    return Walk::Skip;
                }
                if !approved.iter().any(|root| path.starts_with(root)) {
                    paths.push(path.to_path_buf());
                }
                return Walk::Skip;
            }
            Walk::Continue
        },
    )?;
    if let Some(path) = symlink {
        return Err(format!(
            "cannot enforce writable workspace control symlink: {}",
            path.display()
        ));
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn missing_workspace_control_paths(
    cwd: &Path,
    approved: &BTreeSet<PathBuf>,
) -> Vec<SyntheticDirectory> {
    PROTECTED_METADATA_NAMES
        .into_iter()
        .map(|name| cwd.join(name))
        .filter(|path| !path.exists() && !approved.contains(path))
        .map(SyntheticDirectory)
        .collect()
}

#[derive(Clone)]
struct ConcreteDeny {
    access: DeniedAccess,
    path: PathBuf,
}

fn concrete_denies(denies: &[NormalizedDeny], cwd: &Path) -> Result<Vec<ConcreteDeny>, String> {
    let mut by_path = BTreeMap::new();
    for deny in denies {
        let paths = match deny.scope {
            DenyScope::File | DenyScope::Tree => {
                if let Some(path) = &deny.path
                    && path.exists()
                {
                    vec![path.clone()]
                } else {
                    Vec::new()
                }
            }
            DenyScope::Glob => expand_glob(&deny.pattern, cwd)?,
        };
        for path in paths {
            if deny.exempt_roots.iter().any(|root| path.starts_with(root)) {
                continue;
            }
            let path = if path
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                path.canonicalize().map_err(|error| {
                    format!("cannot resolve deny symlink {}: {error}", path.display())
                })?
            } else {
                path
            };
            by_path
                .entry(path)
                .and_modify(|access| *access = merge_denied_access(*access, deny.access))
                .or_insert(deny.access);
        }
    }
    Ok(by_path
        .into_iter()
        .map(|(path, access)| ConcreteDeny { access, path })
        .collect())
}

fn merge_denied_access(left: DeniedAccess, right: DeniedAccess) -> DeniedAccess {
    if left == right {
        left
    } else {
        DeniedAccess::ReadWrite
    }
}

#[cfg(test)]
fn concrete_deny_for_test(access: DeniedAccess, path: PathBuf) -> NormalizedDeny {
    NormalizedDeny {
        access,
        pattern: path.to_string_lossy().into_owned(),
        scope: DenyScope::File,
        path: Some(path),
        exempt_roots: Vec::new(),
    }
}

fn reject_writable_symlink_crossings(
    denies: &[ConcreteDeny],
    writable: &[NormalizedRight],
) -> Result<(), String> {
    for deny in denies {
        let mut current = PathBuf::new();
        for component in deny.path.components() {
            current.push(component.as_os_str());
            let Ok(metadata) = current.symlink_metadata() else {
                break;
            };
            if metadata.file_type().is_symlink()
                && writable
                    .iter()
                    .any(|right| current.starts_with(&right.path))
            {
                return Err(format!(
                    "cannot enforce deny path {} across writable symlink {}",
                    deny.path.display(),
                    current.display()
                ));
            }
        }
    }
    Ok(())
}

fn append_deny(
    args: &mut Vec<String>,
    deny: &ConcreteDeny,
    hidden_file: Option<&File>,
) -> Result<(), String> {
    if !deny.path.exists() {
        return Ok(());
    }
    if matches!(deny.access, DeniedAccess::Read | DeniedAccess::ReadWrite) {
        args.push("--perms".to_owned());
        args.push("000".to_owned());
        if deny.path.is_dir() {
            args.push("--tmpfs".to_owned());
            args.push(path_string(&deny.path));
            args.push("--remount-ro".to_owned());
            args.push(path_string(&deny.path));
        } else {
            let hidden_file = hidden_file.ok_or("hidden file source is unavailable")?;
            args.push("--ro-bind-data".to_owned());
            args.push(hidden_file.as_raw_fd().to_string());
            args.push(path_string(&deny.path));
        }
    } else {
        push_mount(args, "--ro-bind", &deny.path, &deny.path);
    }
    Ok(())
}

fn hidden_file_source() -> Result<File, String> {
    let file = File::open("/dev/null")
        .map_err(|error| format!("cannot open hidden-file source: {error}"))?;
    make_inheritable(&file, "hidden-file source")?;
    Ok(file)
}

fn expand_glob(pattern: &str, cwd: &Path) -> Result<Vec<PathBuf>, String> {
    let regex = Regex::new(&glob_regex(pattern)?).map_err(|error| error.to_string())?;
    let roots = glob_scan_roots(pattern, cwd)?;
    let mut matches = BTreeSet::new();
    let mut visited_entries = 0_usize;
    let mut visited_directories = BTreeSet::new();
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        walk(
            &root,
            0,
            true,
            &mut visited_entries,
            &mut visited_directories,
            &mut |path, file_type| {
                if immutable_store_directory_symlink(path, file_type) {
                    return Walk::Skip;
                }
                if regex.is_match(&path.to_string_lossy()) {
                    matches.insert(path.to_path_buf());
                    if let Ok(canonical) = path.canonicalize() {
                        matches.insert(canonical);
                    }
                }
                Walk::Continue
            },
        )?;
        if matches.len() > MAX_GLOB_MATCHES {
            return Err(format!(
                "deny glob matched more than {MAX_GLOB_MATCHES} paths: {pattern}"
            ));
        }
    }
    Ok(matches.into_iter().collect())
}

fn immutable_store_directory_symlink(path: &Path, file_type: &fs::FileType) -> bool {
    file_type.is_symlink()
        && path
            .canonicalize()
            .is_ok_and(|target| target.is_dir() && target.starts_with(Path::new("/nix/store")))
}

fn glob_scan_roots(pattern: &str, cwd: &Path) -> Result<BTreeSet<PathBuf>, String> {
    let first_glob = pattern
        .char_indices()
        .find_map(|(index, character)| matches!(character, '*' | '?' | '[').then_some(index))
        .ok_or_else(|| format!("glob deny has no metacharacter: {pattern}"))?;
    let prefix = &pattern[..first_glob];
    let end = if prefix.ends_with('/') {
        prefix.len().saturating_sub(1)
    } else {
        prefix.rfind('/').unwrap_or(0)
    };
    let static_root = if end == 0 {
        Path::new("/")
    } else {
        Path::new(&pattern[..end])
    };
    if static_root != Path::new("/") {
        return Ok(BTreeSet::from([static_root.to_path_buf()]));
    }

    // Root-wide startup scans are both costly and misleading. Protocol v4
    // applies filename-pattern denies to the active workspace; fixed hard
    // denies separately protect SSH, cloud, auth, and control paths in HOME.
    Ok(BTreeSet::from([cwd.to_path_buf()]))
}

enum Walk {
    Continue,
    Skip,
}

fn walk(
    directory: &Path,
    depth: usize,
    follow_symlink_directories: bool,
    visited_entries: &mut usize,
    visited_directories: &mut BTreeSet<PathBuf>,
    callback: &mut impl FnMut(&Path, &fs::FileType) -> Walk,
) -> Result<(), String> {
    if depth > MAX_SCAN_DEPTH {
        return Err(format!(
            "filesystem policy scan exceeds depth {MAX_SCAN_DEPTH}"
        ));
    }
    let canonical = directory.canonicalize().map_err(|error| {
        format!(
            "cannot resolve policy root {}: {error}",
            directory.display()
        )
    })?;
    if !visited_directories.insert(canonical) {
        return Ok(());
    }
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("cannot scan policy root {}: {error}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("policy scan failed: {error}"))?;
        *visited_entries += 1;
        if *visited_entries > MAX_SCAN_ENTRIES {
            return Err(format!(
                "filesystem policy scan exceeds {MAX_SCAN_ENTRIES} entries"
            ));
        }
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
        let action = callback(&path, &file_type);
        let directory = file_type.is_dir()
            || (follow_symlink_directories
                && file_type.is_symlink()
                && fs::metadata(&path).is_ok_and(|metadata| metadata.is_dir()));
        if directory && matches!(action, Walk::Continue) {
            walk(
                &path,
                depth + 1,
                follow_symlink_directories,
                visited_entries,
                visited_directories,
                callback,
            )?;
        }
    }
    Ok(())
}

fn glob_regex(pattern: &str) -> Result<String, String> {
    if !pattern.starts_with('/') {
        return Err("deny glob must be absolute".to_owned());
    }
    let mut regex = String::from("^");
    let mut chars = pattern.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
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
            '[' | ']' => return Err("glob character classes are unsupported".to_owned()),
            _ => regex.push_str(&escape(&character.to_string())),
        }
    }
    regex.push('$');
    Ok(regex)
}

fn is_control_root(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|name| name == ".git" || name == ".pi")
}

fn path_depth(path: &Path) -> usize {
    path.components().count()
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn push_mount(args: &mut Vec<String>, operation: &str, source: &Path, target: &Path) {
    args.push(operation.to_owned());
    args.push(path_string(source));
    args.push(path_string(target));
}

#[derive(Clone, Copy)]
struct FilterInstruction {
    code: u16,
    jump_true: u8,
    jump_false: u8,
    value: u32,
}

impl FilterInstruction {
    fn encode(self, output: &mut Vec<u8>) {
        output.extend_from_slice(&self.code.to_ne_bytes());
        output.push(self.jump_true);
        output.push(self.jump_false);
        output.extend_from_slice(&self.value.to_ne_bytes());
    }
}

fn statement(code: u16, value: u32) -> FilterInstruction {
    FilterInstruction {
        code,
        jump_true: 0,
        jump_false: 0,
        value,
    }
}

fn jump(value: u32, jump_true: u8, jump_false: u8) -> FilterInstruction {
    FilterInstruction {
        code: BPF_JMP_JEQ_K,
        jump_true,
        jump_false,
        value,
    }
}

#[cfg(target_arch = "x86_64")]
fn jump_greater_or_equal(value: u32, jump_true: u8, jump_false: u8) -> FilterInstruction {
    FilterInstruction {
        code: BPF_JMP_JGE_K,
        jump_true,
        jump_false,
        value,
    }
}

fn seccomp_program() -> Vec<u8> {
    let errno = SECCOMP_RET_ERRNO | u32::try_from(libc::EPERM).expect("EPERM is positive");
    let mut program = vec![
        statement(BPF_LD_W_ABS, SECCOMP_DATA_ARCH_OFFSET),
        jump(AUDIT_ARCH, 1, 0),
        statement(BPF_RET_K, SECCOMP_RET_KILL_PROCESS),
        statement(BPF_LD_W_ABS, 0),
    ];
    #[cfg(target_arch = "x86_64")]
    program.extend([
        // x32 uses the x86_64 audit architecture with bit 30 set on each
        // syscall number. Deny that ABI so it cannot bypass the native table.
        jump_greater_or_equal(0x4000_0000, 0, 1),
        statement(BPF_RET_K, errno),
    ]);
    for syscall in [
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_io_uring_setup,
        libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register,
        libc::SYS_connect,
        libc::SYS_accept,
        libc::SYS_accept4,
        libc::SYS_bind,
        libc::SYS_listen,
        libc::SYS_getpeername,
        libc::SYS_getsockname,
        libc::SYS_shutdown,
        libc::SYS_sendto,
        libc::SYS_sendmmsg,
        libc::SYS_recvmmsg,
        libc::SYS_getsockopt,
        libc::SYS_setsockopt,
    ] {
        program.push(jump(
            u32::try_from(syscall).expect("syscall number fits u32"),
            0,
            1,
        ));
        program.push(statement(BPF_RET_K, errno));
    }
    for syscall in [libc::SYS_socket, libc::SYS_socketpair] {
        program.push(jump(
            u32::try_from(syscall).expect("syscall number fits u32"),
            0,
            3,
        ));
        program.push(statement(BPF_LD_W_ABS, SECCOMP_DATA_ARGS_OFFSET));
        program.push(jump(
            u32::try_from(libc::AF_UNIX).expect("AF_UNIX fits u32"),
            1,
            0,
        ));
        program.push(statement(BPF_RET_K, errno));
        program.push(statement(BPF_LD_W_ABS, 0));
    }
    program.push(statement(BPF_RET_K, SECCOMP_RET_ALLOW));

    let mut bytes = Vec::with_capacity(program.len() * 8);
    for instruction in program {
        instruction.encode(&mut bytes);
    }
    bytes
}

fn seccomp_file() -> Result<File, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("clock error: {error}"))?
        .as_nanos();
    let path = std::env::temp_dir().join(format!(".pi-seccomp-{}-{nonce}", std::process::id()));
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| format!("cannot create seccomp program: {error}"))?;
    let _ = fs::remove_file(&path);
    file.write_all(&seccomp_program())
        .map_err(|error| format!("cannot write seccomp program: {error}"))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("cannot rewind seccomp program: {error}"))?;
    make_inheritable(&file, "seccomp descriptor")?;
    Ok(file)
}

/// Runs inside a fresh Bubblewrap network namespace. In proxy mode, the bridge
/// itself may connect to the one host proxy socket. The user command receives a
/// seccomp filter that blocks AF_UNIX, so it cannot reuse this launcher to reach
/// other host services. AF_INET stays inside the isolated namespace.
pub fn run_proxy_launcher(arguments: &[String]) -> Result<i32, String> {
    let separator = arguments
        .iter()
        .position(|argument| argument == "--")
        .ok_or("network launcher command separator is missing")?;
    if separator != 2 || arguments.len() <= 3 {
        return Err("network launcher arguments are invalid".to_owned());
    }
    let socket_path = (arguments[0] != "-").then(|| PathBuf::from(&arguments[0]));
    if socket_path.as_ref().is_some_and(|path| {
        !path.is_absolute()
            || !path
                .metadata()
                .is_ok_and(|item| item.file_type().is_socket())
    }) {
        return Err("network launcher proxy socket is invalid".to_owned());
    }
    let allow_local_binding = match arguments[1].as_str() {
        "allow-local-binding" => true,
        "deny-local-binding" => false,
        _ => return Err("network launcher local binding mode is invalid".to_owned()),
    };
    if socket_path.is_none() && !allow_local_binding {
        return Err("network launcher needs a proxy or local binding".to_owned());
    }
    ensure_loopback_up()?;
    if let Some(bridge_socket) = socket_path {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, PROXY_LOOPBACK_PORT))
            .map_err(|error| format!("cannot bind sandbox proxy loopback: {error}"))?;
        std::thread::spawn(move || {
            for incoming in listener.incoming() {
                let Ok(tcp) = incoming else { break };
                let socket = bridge_socket.clone();
                std::thread::spawn(move || {
                    let Ok(unix) = UnixStream::connect(socket) else {
                        return;
                    };
                    let _ = proxy_bidirectional(tcp, unix);
                });
            }
        });
    }

    let mut child = Command::new(&arguments[separator + 1]);
    child.args(&arguments[separator + 2..]);
    // SAFETY: this closure runs after fork and calls only libc before exec.
    unsafe {
        child.pre_exec(move || {
            install_proxy_user_seccomp(allow_local_binding).map_err(std::io::Error::other)
        });
    }
    let status = child
        .status()
        .map_err(|error| format!("cannot start proxied command: {error}"))?;
    Ok(status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(1)))
}

fn ensure_loopback_up() -> Result<(), String> {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(format!(
            "cannot open loopback control socket: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut request = unsafe { std::mem::zeroed::<libc::ifreq>() };
    for (index, byte) in b"lo".iter().copied().enumerate() {
        request.ifr_name[index] = byte as libc::c_char;
    }
    let read = unsafe { libc::ioctl(fd, libc::SIOCGIFFLAGS as libc::Ioctl, &mut request) };
    if read < 0 {
        unsafe { libc::close(fd) };
        return Err(format!(
            "cannot read loopback flags: {}",
            std::io::Error::last_os_error()
        ));
    }
    unsafe {
        request.ifr_ifru.ifru_flags |= libc::IFF_UP as libc::c_short;
    }
    let write = unsafe { libc::ioctl(fd, libc::SIOCSIFFLAGS as libc::Ioctl, &request) };
    unsafe { libc::close(fd) };
    if write < 0 {
        return Err(format!(
            "cannot enable loopback: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn install_proxy_user_seccomp(allow_local_binding: bool) -> Result<(), String> {
    let errno = SECCOMP_RET_ERRNO | u32::try_from(libc::EPERM).expect("EPERM is positive");
    let mut program = vec![
        statement(BPF_LD_W_ABS, SECCOMP_DATA_ARCH_OFFSET),
        jump(AUDIT_ARCH, 1, 0),
        statement(BPF_RET_K, SECCOMP_RET_KILL_PROCESS),
        statement(BPF_LD_W_ABS, 0),
    ];
    #[cfg(target_arch = "x86_64")]
    program.extend([
        jump_greater_or_equal(0x4000_0000, 0, 1),
        statement(BPF_RET_K, errno),
    ]);
    for syscall in [
        libc::SYS_ptrace,
        libc::SYS_process_vm_readv,
        libc::SYS_process_vm_writev,
        libc::SYS_io_uring_setup,
        libc::SYS_io_uring_enter,
        libc::SYS_io_uring_register,
    ] {
        program.push(jump(
            u32::try_from(syscall).expect("syscall fits u32"),
            0,
            1,
        ));
        program.push(statement(BPF_RET_K, errno));
    }
    if !allow_local_binding {
        for syscall in [
            libc::SYS_accept,
            libc::SYS_accept4,
            libc::SYS_bind,
            libc::SYS_listen,
        ] {
            program.push(jump(
                u32::try_from(syscall).expect("syscall fits u32"),
                0,
                1,
            ));
            program.push(statement(BPF_RET_K, errno));
        }
    }
    for syscall in [libc::SYS_socket, libc::SYS_socketpair] {
        program.push(jump(
            u32::try_from(syscall).expect("syscall fits u32"),
            0,
            3,
        ));
        program.push(statement(BPF_LD_W_ABS, SECCOMP_DATA_ARGS_OFFSET));
        program.push(jump(
            u32::try_from(libc::AF_UNIX).expect("AF_UNIX fits u32"),
            0,
            1,
        ));
        program.push(statement(BPF_RET_K, errno));
        program.push(statement(BPF_LD_W_ABS, 0));
    }
    program.push(statement(BPF_RET_K, SECCOMP_RET_ALLOW));
    let mut filters = program
        .into_iter()
        .map(|instruction| libc::sock_filter {
            code: instruction.code,
            jt: instruction.jump_true,
            jf: instruction.jump_false,
            k: instruction.value,
        })
        .collect::<Vec<_>>();
    let filter = libc::sock_fprog {
        len: u16::try_from(filters.len()).map_err(|_| "seccomp filter is too large")?,
        filter: filters.as_mut_ptr(),
    };
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(format!(
            "cannot set no_new_privs: {}",
            std::io::Error::last_os_error()
        ));
    }
    if unsafe { libc::prctl(libc::PR_SET_SECCOMP, libc::SECCOMP_MODE_FILTER, &filter) } != 0 {
        return Err(format!(
            "cannot install proxy seccomp: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

fn proxy_bidirectional(mut tcp: TcpStream, mut unix: UnixStream) -> std::io::Result<()> {
    let mut tcp_reader = tcp.try_clone()?;
    let mut unix_writer = unix.try_clone()?;
    let left = std::thread::spawn(move || {
        let result = std::io::copy(&mut tcp_reader, &mut unix_writer);
        let _ = unix_writer.shutdown(Shutdown::Write);
        result
    });
    let right = std::io::copy(&mut unix, &mut tcp);
    let _ = tcp.shutdown(Shutdown::Write);
    left.join()
        .map_err(|_| std::io::Error::other("proxy bridge thread panicked"))??;
    right?;
    Ok(())
}

fn make_inheritable(file: &File, label: &str) -> Result<(), String> {
    fcntl(file, FcntlArg::F_SETFD(FdFlag::empty()))
        .map_err(|error| format!("cannot make {label} inheritable: {error}"))?;
    Ok(())
}

/// Runs the exact namespace and seccomp pipeline before advertising readiness.
pub fn self_test() -> Result<(), String> {
    if !Path::new(BWRAP).is_file() {
        return Err(format!("fixed Bubblewrap path is unavailable: {BWRAP}"));
    }
    let seccomp = seccomp_file()?;
    let hidden_file = hidden_file_source()?;
    let script = r#"found=; while read -r key value rest; do [ "$key" = "NoNewPrivs:" ] && found=$value; done < /proc/self/status; [ "$found" = 1 ] && [ "$$" -le 2 ] && [ ! -r /etc/passwd ]"#;
    let mut args = base_args();
    args.extend([
        "--perms".to_owned(),
        "000".to_owned(),
        "--ro-bind-data".to_owned(),
        hidden_file.as_raw_fd().to_string(),
        "/etc/passwd".to_owned(),
        "--seccomp".to_owned(),
        seccomp.as_raw_fd().to_string(),
    ]);
    args.extend([
        "--".to_owned(),
        "/bin/sh".to_owned(),
        "-c".to_owned(),
        script.to_owned(),
    ]);
    let output = Command::new(BWRAP)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("cannot start Bubblewrap self-test: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Bubblewrap self-test failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_args_request_all_release_namespaces() {
        let args = base_args();
        for required in [
            "--unshare-user",
            "--unshare-pid",
            "--unshare-net",
            "--unshare-ipc",
            "--unshare-uts",
            "--proc",
            "--dev",
        ] {
            assert!(args.iter().any(|argument| argument == required));
        }
    }

    #[test]
    fn seccomp_program_checks_arch_and_ends_in_allow() {
        let bytes = seccomp_program();
        assert_eq!(bytes.len() % 8, 0);
        assert_eq!(&bytes[4..8], &SECCOMP_DATA_ARCH_OFFSET.to_ne_bytes());
        assert_eq!(&bytes[bytes.len() - 4..], &SECCOMP_RET_ALLOW.to_ne_bytes());
    }

    #[test]
    fn root_globs_scan_only_the_active_workspace() {
        let cwd = Path::new("/work");
        let roots = glob_scan_roots("/**/*.env", cwd).expect("roots");
        assert!(roots.contains(cwd));
        assert!(!roots.contains(Path::new("/")));
    }

    #[test]
    fn env_glob_exemptions_skip_only_managed_cache_descendants() {
        let root = std::env::temp_dir().join(format!(
            "pi-linux-cache-exemption-test-{}",
            std::process::id()
        ));
        let cache = root.join("pi-sandbox");
        let sibling = root.join("pi-sandbox-other");
        fs::create_dir_all(&cache).expect("cache fixture");
        fs::create_dir_all(&sibling).expect("sibling fixture");
        fs::write(cache.join(".env.toml"), "public").expect("cache env fixture");
        fs::write(sibling.join(".env.toml"), "secret").expect("sibling env fixture");
        let deny = NormalizedDeny {
            access: DeniedAccess::ReadWrite,
            pattern: format!("{}/**/.env.*", root.display()),
            scope: DenyScope::Glob,
            path: None,
            exempt_roots: vec![cache.clone()],
        };
        let concrete = concrete_denies(&[deny], &root).expect("concrete denies");
        assert_eq!(concrete.len(), 1);
        assert_eq!(concrete[0].path, sibling.join(".env.toml"));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn duplicate_concrete_denies_merge_to_one_strongest_mount() {
        let path = std::env::temp_dir().join(format!(
            "pi-linux-duplicate-deny-test-{}",
            std::process::id()
        ));
        fs::write(&path, "secret").expect("fixture");
        let denies = vec![
            concrete_deny_for_test(DeniedAccess::Read, path.clone()),
            concrete_deny_for_test(DeniedAccess::ReadWrite, path.clone()),
        ];
        let concrete = concrete_denies(&denies, Path::new("/")).expect("concrete denies");
        assert_eq!(concrete.len(), 1);
        assert_eq!(concrete[0].path, path);
        assert_eq!(concrete[0].access, DeniedAccess::ReadWrite);
        fs::remove_file(&concrete[0].path).expect("cleanup");
    }

    #[test]
    fn concrete_deny_mounts_the_target_of_a_symlink() {
        use std::os::unix::fs::symlink;

        let root =
            std::env::temp_dir().join(format!("pi-linux-deny-symlink-test-{}", std::process::id()));
        fs::create_dir_all(&root).expect("fixture directory");
        let target = root.join("target");
        let link = root.join("link");
        fs::write(&target, "secret").expect("fixture target");
        symlink(&target, &link).expect("fixture symlink");
        let denies = vec![concrete_deny_for_test(DeniedAccess::Read, link)];
        let concrete = concrete_denies(&denies, &root).expect("concrete denies");
        assert_eq!(concrete.len(), 1);
        assert_eq!(
            concrete[0].path,
            target.canonicalize().expect("canonical target")
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn nix_store_directory_symlinks_are_immutable_scan_boundaries() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "pi-linux-store-symlink-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("fixture directory");
        let store_link = root.join("result");
        symlink("/nix/store", &store_link).expect("store symlink");
        let file_type = store_link
            .symlink_metadata()
            .expect("symlink metadata")
            .file_type();
        assert!(immutable_store_directory_symlink(&store_link, &file_type));

        let ordinary_target = root.join("ordinary-target");
        fs::create_dir(&ordinary_target).expect("ordinary target");
        let ordinary_link = root.join("ordinary-link");
        symlink(&ordinary_target, &ordinary_link).expect("ordinary symlink");
        let ordinary_type = ordinary_link
            .symlink_metadata()
            .expect("ordinary metadata")
            .file_type();
        assert!(!immutable_store_directory_symlink(
            &ordinary_link,
            &ordinary_type
        ));
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn glob_scan_follows_directory_symlinks_and_records_the_target() {
        use std::os::unix::fs::symlink;

        let root =
            std::env::temp_dir().join(format!("pi-linux-glob-symlink-test-{}", std::process::id()));
        let workspace = root.join("workspace");
        let target = root.join("target");
        fs::create_dir_all(&workspace).expect("workspace");
        fs::create_dir_all(&target).expect("target");
        let secret = target.join("secret.env");
        fs::write(&secret, "secret").expect("secret");
        symlink(&target, workspace.join("linked")).expect("symlink");
        let pattern = format!("{}/**/*.env", workspace.display());
        let matches = expand_glob(&pattern, &workspace).expect("expand glob");
        assert!(matches.contains(&secret.canonicalize().expect("canonical secret")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn writable_symlink_crossing_is_rejected() {
        use std::os::unix::fs::symlink;

        let root =
            std::env::temp_dir().join(format!("pi-linux-symlink-test-{}", std::process::id()));
        let writable_root = root.join("writable");
        let target = root.join("target");
        fs::create_dir_all(&writable_root).expect("writable root");
        fs::create_dir_all(&target).expect("target");
        let link = writable_root.join("secret");
        symlink(&target, &link).expect("symlink");
        let denies = vec![ConcreteDeny {
            access: DeniedAccess::ReadWrite,
            path: link,
        }];
        let writable = vec![NormalizedRight {
            access: Access::Write,
            path: writable_root,
            scope: PathScope::Tree,
            approved: true,
        }];
        assert!(reject_writable_symlink_crossings(&denies, &writable).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_concrete_deny_rejects_a_broad_write() {
        let writable = vec![NormalizedRight {
            access: Access::Write,
            path: PathBuf::from("/work"),
            scope: PathScope::Tree,
            approved: true,
        }];
        let denies = vec![NormalizedDeny {
            access: DeniedAccess::ReadWrite,
            pattern: "/work/missing-secret".to_owned(),
            scope: DenyScope::Tree,
            path: Some(PathBuf::from("/work/missing-secret")),
            exempt_roots: Vec::new(),
        }];
        assert!(reject_missing_concrete_denies(&denies, &writable).is_err());
    }

    #[test]
    fn deny_mounts_follow_write_mounts() {
        let path = std::env::temp_dir();
        let mut args = base_args();
        push_mount(&mut args, "--bind", &path, &path);
        append_deny(
            &mut args,
            &ConcreteDeny {
                access: DeniedAccess::Write,
                path,
            },
            None,
        )
        .expect("deny");
        let write = args.iter().position(|arg| arg == "--bind").expect("write");
        let deny = args
            .iter()
            .rposition(|arg| arg == "--ro-bind")
            .expect("deny");
        assert!(deny > write);
    }
}
