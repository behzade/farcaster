//! macOS protected-read concealment.
//!
//! Seatbelt returns `EPERM` for a denied read. Some tools treat that as a hard
//! failure even when they only probe an optional file. The injected shim maps
//! protected read paths to `ENOENT`; Seatbelt remains the hard boundary when a
//! process skips or removes the shim.

#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};

use crate::protocol::{DeniedAccess, DenyScope};
use crate::seatbelt::NormalizedDeny;

const MAX_ENCODED_BYTES: usize = 192 * 1024;

/// Resolves the broker-owned launcher and dynamic library.
///
/// # Errors
///
/// Returns an error when either helper is missing, relative, or cannot be
/// resolved to its real path.
pub fn helper_paths() -> Result<[PathBuf; 2], String> {
    Ok([
        helper_path(launcher_path(), "conceal launcher")?,
        helper_path(shim_path(), "conceal shim")?,
    ])
}

/// Wraps a command when the policy has at least one protected read rule.
///
/// # Errors
///
/// Returns an error when the encoded rules exceed the fixed size limit or a
/// broker-owned helper is unavailable.
pub fn wrap_command(
    program: &Path,
    arguments: &[String],
    denies: &[NormalizedDeny],
) -> Result<Option<Vec<String>>, String> {
    let encoded = encode_denies(denies)?;
    if encoded.is_empty() {
        return Ok(None);
    }
    let [launcher, shim] = helper_paths()?;
    let mut command = vec![
        launcher.to_string_lossy().into_owned(),
        shim.to_string_lossy().into_owned(),
        encoded,
        "--".to_owned(),
        program.to_string_lossy().into_owned(),
    ];
    command.extend_from_slice(arguments);
    Ok(Some(command))
}

fn launcher_path() -> &'static str {
    option_env!("PI_CONCEAL_LAUNCHER_PATH").unwrap_or(env!("PI_CONCEAL_LAUNCHER_BUILD_PATH"))
}

fn shim_path() -> &'static str {
    option_env!("PI_CONCEAL_SHIM_PATH").unwrap_or(env!("PI_CONCEAL_SHIM_BUILD_PATH"))
}

fn helper_path(value: &str, label: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if !path.is_absolute() || !path.is_file() {
        return Err(format!("{label} is unavailable: {}", path.display()));
    }
    path.canonicalize()
        .map_err(|error| format!("cannot resolve {label} {}: {error}", path.display()))
}

fn encode_denies(denies: &[NormalizedDeny]) -> Result<String, String> {
    let mut encoded = String::new();
    for deny in denies {
        if !matches!(deny.access, DeniedAccess::Read | DeniedAccess::ReadWrite) {
            continue;
        }
        if !encoded.is_empty() {
            encoded.push(',');
        }
        encoded.push(match deny.scope {
            DenyScope::File => 'f',
            DenyScope::Tree => 't',
            DenyScope::Glob => 'g',
        });
        encoded.push(':');
        for byte in deny.pattern.as_bytes() {
            use std::fmt::Write as _;
            write!(encoded, "{byte:02x}").expect("writing to a String cannot fail");
        }
        if encoded.len() > MAX_ENCODED_BYTES {
            return Err(format!(
                "protected read policy exceeds {MAX_ENCODED_BYTES} encoded bytes"
            ));
        }
    }
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_carries_each_read_scope_and_omits_write_only_denies() {
        let denies = vec![
            NormalizedDeny {
                access: DeniedAccess::Read,
                pattern: "/work/.env".to_owned(),
                scope: DenyScope::File,
                path: Some(PathBuf::from("/work/.env")),
            },
            NormalizedDeny {
                access: DeniedAccess::ReadWrite,
                pattern: "/home/user/.ssh".to_owned(),
                scope: DenyScope::Tree,
                path: Some(PathBuf::from("/home/user/.ssh")),
            },
            NormalizedDeny {
                access: DeniedAccess::Read,
                pattern: "/**/.env.*".to_owned(),
                scope: DenyScope::Glob,
                path: None,
            },
            NormalizedDeny {
                access: DeniedAccess::Write,
                pattern: "/work/visible.txt".to_owned(),
                scope: DenyScope::File,
                path: Some(PathBuf::from("/work/visible.txt")),
            },
        ];

        let encoded = encode_denies(&denies).expect("encoded denies");
        assert!(encoded.starts_with("f:2f776f726b2f2e656e76,t:"));
        assert!(encoded.contains(",g:2f2a2a2f2e656e762e2a"));
        assert!(!encoded.contains("76697369626c652e747874"));
        assert!(!encoded.contains("/work/.env"));
    }
}
