use super::git_head_contents;
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
