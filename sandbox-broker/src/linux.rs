//! Linux Bubblewrap policy and launcher preparation.
//!
//! Adapted from OpenAI Codex commit
//! `65ae4c26e088913176a50d6daeb742d00942caee`, chiefly
//! `codex-rs/linux-sandbox/src/{bwrap.rs,landlock.rs,linux_run_main.rs}`.
//! Pi keeps only protocol-v1 foreground execution, uses a compile-time fixed
//! Bubblewrap path, and passes a small reviewed seccomp filter directly to
//! Bubblewrap instead of re-entering the broker.

#![cfg(target_os = "linux")]

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use nix::fcntl::{FcntlArg, FdFlag, fcntl};
use regex_lite::{Regex, escape};

use crate::protocol::{Access, DeniedAccess, DenyScope, PathScope};
use crate::seatbelt::{NormalizedDeny, NormalizedRight};
use crate::validation::ValidatedExec;

pub const BWRAP: &str = match option_env!("PI_BWRAP_PATH") {
    Some(path) => path,
    None => "/usr/bin/bwrap",
};

const MAX_SCAN_ENTRIES: usize = 200_000;
const MAX_GLOB_MATCHES: usize = 8_192;
const MAX_SCAN_DEPTH: usize = 64;
const PROTECTED_METADATA_NAMES: [&str; 2] = [".git", ".pi"];

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
        return Err("Linux protocol v2 requires an explicit read right for /".to_owned());
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
    let seccomp = seccomp_file()?;

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

    args.push("--seccomp".to_owned());
    args.push(seccomp.as_raw_fd().to_string());
    args.push("--chdir".to_owned());
    args.push(path_string(&request.cwd));
    args.push("--".to_owned());
    args.extend_from_slice(command);

    let mut resources = hidden_file.into_iter().collect::<Vec<_>>();
    resources.push(seccomp);
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
    let mut result = Vec::new();
    for deny in denies {
        match deny.scope {
            DenyScope::File | DenyScope::Tree => {
                if let Some(path) = &deny.path
                    && path.exists()
                {
                    result.push(ConcreteDeny {
                        access: deny.access,
                        path: path.clone(),
                    });
                }
            }
            DenyScope::Glob => {
                for path in expand_glob(&deny.pattern, cwd)? {
                    result.push(ConcreteDeny {
                        access: deny.access,
                        path,
                    });
                }
            }
        }
    }
    result.sort_by(|left, right| left.path.cmp(&right.path));
    result.dedup_by(|left, right| left.access == right.access && left.path == right.path);
    Ok(result)
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
            &mut |path, _| {
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

    // Root-wide startup scans are both costly and misleading. Protocol v2
    // protects the two user-controlled areas where agent secrets live. The
    // trusted host user is outside the threat model, so files created after
    // this snapshot are not treated as hostile host races.
    let mut roots = BTreeSet::from([cwd.to_path_buf()]);
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        if home.is_absolute() && home.is_dir() {
            roots.insert(
                home.canonicalize()
                    .map_err(|error| format!("cannot canonicalize HOME for glob scan: {error}"))?,
            );
        }
    }
    Ok(roots)
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
    fn root_globs_scan_workspace_and_home_not_the_host_root() {
        let cwd = Path::new("/work");
        let roots = glob_scan_roots("/**/*.env", cwd).expect("roots");
        assert!(roots.contains(cwd));
        assert!(!roots.contains(Path::new("/")));
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
