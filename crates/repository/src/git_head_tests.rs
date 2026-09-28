use super::{git_head_contents, git_head_contents_with_options};
use std::process::Command;

struct Project(tempfile::TempDir);

impl Project {
    fn new() -> Self {
        Self(
            tempfile::tempdir_in(
                std::env::temp_dir()
                    .canonicalize()
                    .expect("test operation should succeed"),
            )
            .expect("test operation should succeed"),
        )
    }

    fn git(&self, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(self.0.path())
            .args(args)
            .output()
            .expect("test operation should succeed");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[cfg(unix)]
fn executable(project: &Project, name: &str, script: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let path = project.0.path().join(name);
    std::fs::write(&path, script).expect("write fake Git");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("make fake Git executable");
    path
}

#[cfg(unix)]
#[test]
fn head_base_uses_configured_git_and_clears_inherited_routing() {
    const CHILD_PATH: &str = "FARCASTER_HEAD_BASE_TEST_PATH";
    if let Some(path) = std::env::var_os(CHILD_PATH) {
        assert_eq!(
            git_head_contents(std::path::Path::new(&path)).expect("read sanitized HEAD"),
            b"original\n"
        );
        return;
    }
    let project = Project::new();
    project.git(&["init", "-q"]);
    let path = project.0.path().join("file");
    std::fs::write(&path, "original\n").expect("write fixture");
    project.git(&["add", "."]);
    project.git(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-qm",
        "base",
    ]);
    let script = executable(
        &project,
        "configured-git",
        "#!/bin/sh\nset -eu\ntest -z \"${GIT_DIR+x}${GIT_WORK_TREE+x}${GIT_INDEX_FILE+x}\"\ntest \"$GIT_NO_LAZY_FETCH\" = 1\nprintf called >> configured-called\nexec git \"$@\"\n",
    );
    let binary = std::env::current_exe().expect("test executable");
    #[cfg(target_os = "macos")]
    let mut child = {
        let mut command = Command::new(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/run-macos.sh"),
        );
        command.arg(&binary);
        command
    };
    #[cfg(not(target_os = "macos"))]
    let mut child = Command::new(&binary);
    let output = child
        .args([
            "--exact",
            "git_head::tests::head_base_uses_configured_git_and_clears_inherited_routing",
            "--nocapture",
        ])
        .env(CHILD_PATH, &path)
        .env("FARCASTER_GIT", script)
        .env("GIT_DIR", project.0.path().join("wrong.git"))
        .env("GIT_WORK_TREE", project.0.path().join("wrong-worktree"))
        .env("GIT_INDEX_FILE", project.0.path().join("wrong-index"))
        .output()
        .expect("run child with inherited Git routing");
    assert!(output.status.success(), "{output:?}");
    assert!(project.0.path().join("configured-called").exists());
}

#[cfg(unix)]
#[test]
fn head_base_rejects_truncated_output_and_times_out() {
    use crate::adapter::RepositoryOptions;
    use std::time::Duration;

    let project = Project::new();
    let path = project.0.path().join("file");
    let oversized = executable(
        &project,
        "oversized-git",
        "#!/bin/sh\nif [ \"$1\" = rev-parse ]; then pwd; else printf '%01024d' 0; fi\n",
    );
    let error = git_head_contents_with_options(
        &path,
        RepositoryOptions {
            git_executable: oversized.into_os_string(),
            output_limit: 512,
            ..RepositoryOptions::default()
        },
    )
    .expect_err("truncated base must not become a valid diff");
    assert!(
        error.contains("output exceeded repository limit"),
        "{error}"
    );
    let slow = executable(&project, "slow-git", "#!/bin/sh\nsleep 5\n");
    let error = git_head_contents_with_options(
        &path,
        RepositoryOptions {
            git_executable: slow.into_os_string(),
            timeout: Duration::from_millis(20),
            ..RepositoryOptions::default()
        },
    )
    .expect_err("HEAD command must have a deadline");
    assert!(error.contains("timed out"), "{error}");
}

#[test]
fn head_base_handles_nested_added_deleted_and_unborn_files() {
    let project = Project::new();
    project.git(&["init", "-q"]);
    let path = project.0.path().join("nested/it's | file.rs");
    std::fs::create_dir(path.parent().expect("test operation should succeed"))
        .expect("test operation should succeed");
    std::fs::write(&path, "original\n").expect("test operation should succeed");
    assert_eq!(
        git_head_contents(&path).expect("test operation should succeed"),
        b""
    );
    project.git(&["add", "."]);
    project.git(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-qm",
        "base",
    ]);
    std::fs::write(&path, "changed\n").expect("test operation should succeed");
    assert_eq!(
        git_head_contents(&path).expect("test operation should succeed"),
        b"original\n"
    );
    let added = project.0.path().join("nested/added.rs");
    std::fs::write(&added, "new\n").expect("test operation should succeed");
    assert_eq!(
        git_head_contents(&added).expect("test operation should succeed"),
        b""
    );
    project.git(&["add", "."]);
    assert_eq!(
        git_head_contents(&added).expect("test operation should succeed"),
        b""
    );
    std::fs::remove_file(&added).expect("test operation should succeed");
    std::fs::remove_file(&path).expect("test operation should succeed");
    std::fs::remove_dir(path.parent().expect("test operation should succeed"))
        .expect("test operation should succeed");
    assert_eq!(
        git_head_contents(&path).expect("test operation should succeed"),
        b"original\n"
    );
    let outside = Project::new();
    assert!(git_head_contents(&outside.0.path().join("file.rs")).is_err());
}
