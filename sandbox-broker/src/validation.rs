use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::protocol::ExecRequest;
use crate::seatbelt::{HardPolicy, NormalizedDeny, NormalizedRight, normalize_policy};

const MAX_ID_BYTES: usize = 256;
const MAX_ARGS: usize = 4096;
const MAX_ARG_ENV_BYTES: usize = 512 * 1024;
const MAX_ENV_ENTRIES: usize = 1024;
pub const MAX_OUTPUT_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug)]
pub struct ValidatedExec {
    pub id: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub timeout_ms: Option<u64>,
    pub output_limit_bytes: u64,
    pub rights: Vec<NormalizedRight>,
    pub denies: Vec<NormalizedDeny>,
}

/// Validates a complete request before the broker starts a child.
///
/// # Errors
///
/// Returns an error for malformed IDs, commands, environment, paths, or policy.
pub fn validate_exec(request: ExecRequest, hard: &HardPolicy) -> Result<ValidatedExec, String> {
    validate_text("command ID", &request.id, MAX_ID_BYTES)?;
    if request.id.trim().is_empty() {
        return Err("command ID is empty".to_owned());
    }
    if request.command.args.len() > MAX_ARGS {
        return Err("command has too many arguments".to_owned());
    }
    validate_text("program", &request.command.program, MAX_ARG_ENV_BYTES)?;
    let program = existing_absolute_file(Path::new(&request.command.program), "program")?;
    let cwd = existing_absolute_directory(Path::new(&request.cwd), "cwd")?;
    for arg in &request.command.args {
        validate_text("argument", arg, MAX_ARG_ENV_BYTES)?;
    }
    if request.env.len() > MAX_ENV_ENTRIES {
        return Err("environment has too many entries".to_owned());
    }
    let mut total_bytes = request.command.program.len();
    for arg in &request.command.args {
        total_bytes = total_bytes.saturating_add(arg.len());
    }
    for (name, value) in &request.env {
        if !is_env_name(name) {
            return Err(format!("invalid environment name: {name}"));
        }
        validate_text("environment value", value, MAX_ARG_ENV_BYTES)?;
        total_bytes = total_bytes
            .saturating_add(name.len())
            .saturating_add(value.len());
    }
    if total_bytes > MAX_ARG_ENV_BYTES {
        return Err("command arguments and environment are too large".to_owned());
    }
    let output_limit_bytes = request.policy.output_limit_bytes;
    if output_limit_bytes == 0 || output_limit_bytes > MAX_OUTPUT_BYTES {
        return Err(format!(
            "output limit must be between 1 and {MAX_OUTPUT_BYTES} bytes"
        ));
    }
    if request.timeout_ms == Some(0) {
        return Err("timeout must be positive".to_owned());
    }
    if request
        .timeout_ms
        .is_some_and(|timeout| timeout > 24 * 60 * 60 * 1000)
    {
        return Err("timeout exceeds 24 hours".to_owned());
    }
    let (rights, denies) = normalize_policy(&request.policy, hard)?;
    Ok(ValidatedExec {
        id: request.id,
        program,
        args: request.command.args,
        cwd,
        env: request.env,
        timeout_ms: request.timeout_ms,
        output_limit_bytes,
        rights,
        denies,
    })
}

fn validate_text(label: &str, value: &str, maximum: usize) -> Result<(), String> {
    if value.len() > maximum {
        return Err(format!("{label} is too large"));
    }
    if value.contains('\0') {
        return Err(format!("{label} contains NUL"));
    }
    Ok(())
}

fn existing_absolute_file(path: &Path, label: &str) -> Result<PathBuf, String> {
    let path = canonical_absolute(path, label)?;
    if !path.is_file() {
        return Err(format!("{label} is not a file: {}", path.display()));
    }
    Ok(path)
}

fn existing_absolute_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    let path = canonical_absolute(path, label)?;
    if !path.is_dir() {
        return Err(format!("{label} is not a directory: {}", path.display()));
    }
    Ok(path)
}

fn canonical_absolute(path: &Path, label: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("{label} must be absolute"));
    }
    path.canonicalize()
        .map_err(|error| format!("cannot canonicalize {label} {}: {error}", path.display()))
}

fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_names_are_narrow() {
        assert!(is_env_name("PATH"));
        assert!(is_env_name("LC_ALL"));
        assert!(!is_env_name("bad-name"));
        assert!(!is_env_name("1BAD"));
        assert!(!is_env_name(""));
    }
}
