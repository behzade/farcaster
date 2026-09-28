use std::path::{Path, PathBuf};

use crate::adapter::{RepositoryOptions, process::CommandRunner};

pub fn git_head_contents(path: &Path) -> Result<Vec<u8>, String> {
    git_head_contents_with_options(path, RepositoryOptions::default())
}

fn git_head_contents_with_options(
    path: &Path,
    options: RepositoryOptions,
) -> Result<Vec<u8>, String> {
    let parent = path
        .ancestors()
        .skip(1)
        .find(|parent| parent.is_dir())
        .ok_or("File has no parent directory")?;
    let runner = CommandRunner::new(options.timeout, options.output_limit, options.environment);
    let git = |directory: &Path, args: &[&std::ffi::OsStr]| {
        let arguments = args
            .iter()
            .map(|arg| arg.to_os_string())
            .collect::<Vec<_>>();
        let output = runner
            .run(&options.git_executable, &arguments, directory)
            .map_err(|error| format!("Read Git HEAD: {error}"))?;
        if output.stdout_truncated {
            return Err("Read Git HEAD: output exceeded repository limit".to_owned());
        }
        Ok(output)
    };
    let root = git(parent, &["rev-parse".as_ref(), "--show-toplevel".as_ref()])?;
    if !root.status.success() {
        return Err(String::from_utf8_lossy(&root.stderr).trim().to_owned());
    }
    let root = PathBuf::from(String::from_utf8_lossy(&root.stdout).trim_end());
    let relative = path
        .strip_prefix(&root)
        .map_err(|error| error.to_string())?;
    let revision = format!("HEAD:{}", relative.to_string_lossy());
    let contents = git(&root, &["show".as_ref(), revision.as_ref()])?;
    if contents.status.success() {
        return Ok(contents.stdout);
    }
    // Added/untracked files and repositories with no commits have an empty base.
    let head = git(
        &root,
        &["rev-parse".as_ref(), "--verify".as_ref(), "HEAD".as_ref()],
    )?;
    let tree = git(
        &root,
        &[
            "ls-tree".as_ref(),
            "HEAD".as_ref(),
            "--".as_ref(),
            relative.as_os_str(),
        ],
    )?;
    if !head.status.success() || (tree.status.success() && tree.stdout.is_empty()) {
        return Ok(Vec::new());
    }
    Err(String::from_utf8_lossy(&contents.stderr).trim().to_owned())
}

#[cfg(test)]
#[path = "git_head_tests.rs"]
mod tests;
