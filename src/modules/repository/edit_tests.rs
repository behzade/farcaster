use super::*;
use std::collections::BTreeSet;

struct EditRepo {
    temp: TestDirectory,
    backend: RepositoryBackend,
}

impl EditRepo {
    fn new(kind: RepositoryKind) -> Self {
        let temp = TestDirectory::new("edit");
        let root = temp.path().join("repo");
        let home = temp.path().join("home");
        let config = temp.path().join("config");
        fs::create_dir_all(&root).expect("test operation should succeed");
        fs::create_dir_all(&home).expect("test operation should succeed");
        fs::create_dir_all(&config).expect("test operation should succeed");
        let preference = match kind {
            RepositoryKind::Git => {
                run_git(&root, &home, &config, &["init"]);
                run_git(
                    &root,
                    &home,
                    &config,
                    &["config", "user.name", "Review Test"],
                );
                run_git(
                    &root,
                    &home,
                    &config,
                    &["config", "user.email", "review@example.invalid"],
                );
                BackendPreference::Git
            }
            RepositoryKind::Jujutsu => {
                run_jj(&root, &home, &config, &["git", "init"]);
                BackendPreference::Jujutsu
            }
        };
        let options = RepositoryOptions {
            environment: isolated_environment(&home, &config),
            ..RepositoryOptions::default()
        };
        let backend = RepositoryBackend::discover_with_options(&root, preference, options)
            .expect("test operation should succeed")
            .expect("test operation should succeed");
        Self { temp, backend }
    }

    fn root(&self) -> &Path {
        &self.backend.location.workspace_root
    }
    fn write(&self, path: &str, text: impl AsRef<[u8]>) {
        fs::write(self.root().join(path), text).expect("test operation should succeed");
    }
    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.root().join(path)).expect("test operation should succeed")
    }

    fn command(&self, args: &[&str]) -> String {
        let output = Command::new(self.backend.executable())
            .args(args)
            .current_dir(self.root())
            .envs(isolated_environment(
                &self.temp.path().join("home"),
                &self.temp.path().join("config"),
            ))
            .output()
            .expect("test operation should succeed");
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("test operation should succeed")
    }

    fn review(&self, paths: &[&str]) -> RepositoryEditReview {
        let snapshot = self
            .backend
            .snapshot()
            .expect("test operation should succeed");
        self.backend
            .prepare_edit(&snapshot, &paths.iter().map(PathBuf::from).collect())
            .expect("test operation should succeed")
    }

    fn base(&self) {
        self.write("selected", "base\n");
        self.write("other", "base\n");
        if self.backend.location.kind == RepositoryKind::Git {
            self.command(&["add", "."]);
        }
        self.command(&["commit", "-m", "base"]);
    }
}

#[test]
fn git_commit_selected_includes_working_contents_and_preserves_other_staging() {
    let repo = EditRepo::new(RepositoryKind::Git);
    repo.base();
    repo.write("selected", "staged\n");
    repo.write("other", "other staged\n");
    repo.command(&["add", "."]);
    repo.write("selected", "working\n");
    repo.write("other", "other working\n");
    repo.write(":(glob)* new", "new\n");
    let review = repo.review(&["selected", ":(glob)* new"]);
    repo.backend
        .apply_edit(&review, RepositoryEdit::Commit, "feat: chosen files")
        .expect("test operation should succeed");
    assert_eq!(repo.command(&["show", "HEAD:selected"]), "working\n");
    assert_eq!(repo.command(&["show", "HEAD::(glob)* new"]), "new\n");
    assert_eq!(repo.command(&["show", "HEAD:other"]), "base\n");
    assert_eq!(repo.command(&["show", ":other"]), "other staged\n");
    assert_eq!(repo.read("other"), "other working\n");
    assert_eq!(
        repo.command(&["diff", "--cached", "--name-only"]),
        "other\n"
    );
}

#[test]
fn git_review_rejects_same_status_binary_edits_and_empty_message() {
    let repo = EditRepo::new(RepositoryKind::Git);
    repo.base();
    repo.write("selected", b"\0one");
    let review = repo.review(&["selected"]);
    assert!(
        repo.backend
            .apply_edit(&review, RepositoryEdit::Commit, " \n")
            .is_err()
    );
    repo.write("selected", b"\0two");
    for action in [RepositoryEdit::Commit, RepositoryEdit::Discard] {
        assert!(matches!(
            repo.backend.apply_edit(&review, action, "message"),
            Err(RepositoryError::StaleSnapshot)
        ));
    }
    assert_eq!(
        fs::read(repo.root().join("selected")).expect("test operation should succeed"),
        b"\0two"
    );
}

#[test]
fn git_discard_restores_both_layers_and_does_not_touch_other_files() {
    let repo = EditRepo::new(RepositoryKind::Git);
    repo.base();
    repo.write("selected", "staged\n");
    repo.command(&["add", "selected"]);
    repo.write("selected", "working\n");
    repo.write("other", "keep\n");
    let review = repo.review(&["selected"]);
    repo.backend
        .apply_edit(&review, RepositoryEdit::Discard, "")
        .expect("test operation should succeed");
    assert_eq!(repo.read("selected"), "base\n");
    assert_eq!(repo.command(&["show", ":selected"]), "base\n");
    assert_eq!(repo.read("other"), "keep\n");
    fs::remove_file(repo.root().join("selected")).expect("test operation should succeed");
    repo.backend
        .apply_edit(&repo.review(&["selected"]), RepositoryEdit::Discard, "")
        .expect("test operation should succeed");
    assert_eq!(repo.read("selected"), "base\n");
}

#[test]
fn git_discard_handles_renames_and_new_files() {
    let repo = EditRepo::new(RepositoryKind::Git);
    repo.base();
    repo.command(&["mv", "selected", "renamed"]);
    let review = repo.review(&["renamed"]);
    assert!(review.paths().contains(&PathBuf::from("selected")));
    repo.backend
        .apply_edit(&review, RepositoryEdit::Discard, "")
        .expect("test operation should succeed");
    assert_eq!(repo.read("selected"), "base\n");
    assert!(!repo.root().join("renamed").exists());
    for staged in [false, true] {
        repo.write("new", "new\n");
        if staged {
            repo.command(&["add", "new"]);
        }
        repo.backend
            .apply_edit(&repo.review(&["new"]), RepositoryEdit::Discard, "")
            .expect("test operation should succeed");
        assert!(!repo.root().join("new").exists());
    }
}

#[test]
fn git_initial_commit_selects_only_chosen_new_file() {
    let repo = EditRepo::new(RepositoryKind::Git);
    repo.write("selected", "new\n");
    repo.write("other", "keep\n");
    repo.command(&["add", "other"]);
    repo.backend
        .apply_edit(
            &repo.review(&["selected"]),
            RepositoryEdit::Commit,
            "initial",
        )
        .expect("test operation should succeed");
    assert_eq!(
        repo.command(&["ls-tree", "--name-only", "HEAD"]),
        "selected\n"
    );
    assert_eq!(
        repo.command(&["diff", "--cached", "--name-only"]),
        "other\n"
    );
}

#[test]
fn review_rejects_empty_selection_and_paths_outside_project() {
    let repo = EditRepo::new(RepositoryKind::Git);
    repo.base();
    let snapshot = repo
        .backend
        .snapshot()
        .expect("test operation should succeed");
    for selected in [BTreeSet::new(), BTreeSet::from([PathBuf::from("../other")])] {
        assert!(repo.backend.prepare_edit(&snapshot, &selected).is_err());
    }
}

#[test]
fn jj_commit_selected_and_discard_keep_other_changes() {
    let repo = EditRepo::new(RepositoryKind::Jujutsu);
    repo.base();
    repo.write("selected", "chosen\n");
    repo.write("other", "keep\n");
    repo.write("a|b.txt", "literal\n");
    let review = repo.review(&["selected", "a|b.txt"]);
    repo.backend
        .apply_edit(&review, RepositoryEdit::Commit, "chosen files")
        .expect("test operation should succeed");
    assert_eq!(
        repo.command(&["file", "show", "-r", "@-", "selected"]),
        "chosen\n"
    );
    assert_eq!(
        repo.command(&["file", "show", "-r", "@-", "other"]),
        "base\n"
    );
    assert_eq!(repo.read("other"), "keep\n");
    repo.backend
        .apply_edit(&repo.review(&["other"]), RepositoryEdit::Discard, "")
        .expect("test operation should succeed");
    assert_eq!(repo.read("other"), "base\n");
    assert_eq!(repo.read("selected"), "chosen\n");
}

#[test]
fn jj_review_rejects_changes_after_review() {
    let repo = EditRepo::new(RepositoryKind::Jujutsu);
    repo.base();
    repo.write("selected", "reviewed\n");
    let review = repo.review(&["selected"]);
    repo.write("selected", "later\n");
    assert!(matches!(
        repo.backend
            .apply_edit(&review, RepositoryEdit::Discard, ""),
        Err(RepositoryError::StaleSnapshot)
    ));
    assert_eq!(repo.read("selected"), "later\n");
}

#[cfg(unix)]
#[test]
fn git_failed_commit_preserves_contents_and_unrelated_index_entries() {
    use std::os::unix::fs::PermissionsExt as _;
    let repo = EditRepo::new(RepositoryKind::Git);
    repo.base();
    repo.write("selected", "keep chosen\n");
    repo.write("new", "keep new\n");
    repo.write("other", "keep staged\n");
    repo.command(&["add", "other"]);
    let head = repo.command(&["rev-parse", "HEAD"]);
    let hook = repo.root().join(".git/hooks/pre-commit");
    fs::write(&hook, "#!/bin/sh\nexit 1\n").expect("test operation should succeed");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755))
        .expect("test operation should succeed");
    assert!(
        repo.backend
            .apply_edit(
                &repo.review(&["selected", "new"]),
                RepositoryEdit::Commit,
                "must fail"
            )
            .is_err()
    );
    assert_eq!(repo.command(&["rev-parse", "HEAD"]), head);
    assert_eq!(repo.read("selected"), "keep chosen\n");
    assert_eq!(repo.read("new"), "keep new\n");
    assert_eq!(repo.command(&["show", ":other"]), "keep staged\n");
}

#[test]
fn scoped_commit_leaves_the_outside_end_of_a_rename_staged() {
    let mut repo = EditRepo::new(RepositoryKind::Git);
    repo.base();
    fs::create_dir(repo.root().join("nested")).expect("test operation should succeed");
    repo.command(&["mv", "selected", "nested/selected"]);
    repo.backend.location.project_root = repo.root().join("nested");
    let snapshot = repo
        .backend
        .snapshot()
        .expect("test operation should succeed");
    let review = repo
        .backend
        .prepare_edit(
            &snapshot,
            &BTreeSet::from([PathBuf::from("nested/selected")]),
        )
        .expect("test operation should succeed");
    assert_eq!(review.paths(), &[PathBuf::from("nested/selected")]);
    repo.backend
        .apply_edit(&review, RepositoryEdit::Commit, "add nested file")
        .expect("test operation should succeed");
    assert_eq!(repo.command(&["show", "HEAD:selected"]), "base\n");
    assert_eq!(
        repo.command(&["diff", "--cached", "--name-only"]),
        "selected\n"
    );
    assert_eq!(repo.read("nested/selected"), "base\n");
}

#[cfg(unix)]
#[test]
fn symlink_discard_does_not_follow_the_target() {
    let repo = EditRepo::new(RepositoryKind::Git);
    repo.base();
    let outside = repo.temp.path().join("outside");
    fs::write(&outside, "keep outside\n").expect("test operation should succeed");
    std::os::unix::fs::symlink(&outside, repo.root().join("link"))
        .expect("test operation should succeed");
    repo.backend
        .apply_edit(&repo.review(&["link"]), RepositoryEdit::Discard, "")
        .expect("test operation should succeed");
    assert_eq!(
        fs::read_to_string(&outside).expect("test operation should succeed"),
        "keep outside\n"
    );
    assert!(fs::symlink_metadata(repo.root().join("link")).is_err());
}
