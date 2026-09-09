use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub(super) fn head_contents(path: &Path) -> Result<Vec<u8>, String> {
    let parent = path
        .ancestors()
        .skip(1)
        .find(|parent| parent.is_dir())
        .ok_or("File has no parent directory")?;
    let git = |directory: &Path, args: &[&std::ffi::OsStr]| {
        Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(args)
            .output()
            .map_err(|error| format!("Read Git HEAD: {error}"))
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
#[path = "neovim_diff_tests.rs"]
mod tests;
