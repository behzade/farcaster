use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use super::*;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("pi-repository-{label}-{}-{id}", std::process::id()));
        fs::create_dir_all(&path).expect("create test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _remove_result = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn auto_uses_deepest_marker_and_jj_wins_only_a_tie() {
    let temp = TestDirectory::new("discovery");
    fs::create_dir(temp.path().join(".git")).expect("create Git marker");
    fs::create_dir(temp.path().join(".jj")).expect("create JJ marker");
    let nested = temp.path().join("nested/project");
    fs::create_dir_all(&nested).expect("create nested project");
    fs::create_dir(nested.parent().expect("nested parent").join(".git"))
        .expect("create nested Git marker");

    let root = RepositoryBackend::discover(temp.path(), BackendPreference::Auto)
        .expect("discover root")
        .expect("root repository");
    assert_eq!(root.location.kind, RepositoryKind::Jujutsu);

    let nested_backend = RepositoryBackend::discover(&nested, BackendPreference::Auto)
        .expect("discover nested")
        .expect("nested repository");
    assert_eq!(nested_backend.location.kind, RepositoryKind::Git);
    assert_eq!(
        nested_backend.location.workspace_root,
        nested
            .parent()
            .expect("nested parent")
            .canonicalize()
            .expect("canonical nested root")
    );
}

#[test]
fn forced_backend_never_falls_back() {
    let temp = TestDirectory::new("forced");
    fs::create_dir(temp.path().join(".jj")).expect("create JJ marker");

    let error = RepositoryBackend::discover(temp.path(), BackendPreference::Git)
        .expect_err("forced Git must not use JJ");
    assert!(matches!(
        error,
        RepositoryError::BackendUnavailable {
            kind: RepositoryKind::Git,
            ..
        }
    ));

    fs::create_dir(temp.path().join(".git")).expect("create Git marker");
    let git = RepositoryBackend::discover(temp.path(), BackendPreference::Git)
        .expect("discover forced Git")
        .expect("Git repository");
    let jj = RepositoryBackend::discover(temp.path(), BackendPreference::Jujutsu)
        .expect("discover forced JJ")
        .expect("JJ repository");
    assert_eq!(git.location.kind, RepositoryKind::Git);
    assert_eq!(jj.location.kind, RepositoryKind::Jujutsu);
}

#[test]
fn no_repository_is_distinct_from_failure() {
    let temp = TestDirectory::new("none");
    let result = RepositoryBackend::discover(temp.path(), BackendPreference::Auto)
        .expect("marker scan should succeed");
    assert!(result.is_none());
}

#[cfg(unix)]
#[test]
fn stable_keys_do_not_use_lossy_path_display() {
    use std::os::unix::ffi::OsStringExt as _;

    let temp = TestDirectory::new("keys");
    let location = RepositoryLocation {
        kind: RepositoryKind::Git,
        workspace_root: temp.path().to_path_buf(),
        project_root: temp.path().to_path_buf(),
    };
    let first = change(
        &location,
        SnapshotToken::Git(Arc::from([])),
        PathBuf::from(OsString::from_vec(vec![0xff])),
        None,
        ChangeLayer::GitUntracked,
        ChangeKind::Untracked,
    )
    .expect("first target");
    let second = change(
        &location,
        SnapshotToken::Git(Arc::from([])),
        PathBuf::from(OsString::from_vec(vec![0xfe])),
        None,
        ChangeLayer::GitUntracked,
        ChangeKind::Untracked,
    )
    .expect("second target");
    assert_eq!(
        first.target.relative_path.to_string_lossy(),
        second.target.relative_path.to_string_lossy()
    );
    assert_eq!(first.target.layer, ChangeLayer::GitUntracked);
    assert_eq!(first.target.kind.status_label(), "?");
    assert_ne!(first.target.key, second.target.key);
}

#[test]
fn backend_preferences_have_stable_storage_values() {
    assert_eq!(BackendPreference::Auto.as_str(), "auto");
    assert_eq!(BackendPreference::Git.as_str(), "git");
    assert_eq!(BackendPreference::Jujutsu.as_str(), "jj");
    assert_eq!(
        "jj".parse::<BackendPreference>().expect("JJ preference"),
        BackendPreference::Jujutsu
    );
}

#[test]
fn diff_results_count_text_and_mark_binary_counts_unknown() {
    assert_eq!(
        patch_counts("--- a/x\n+++ b/x\n-old\n+new\n+more\n"),
        (Some(2), Some(1))
    );
    assert_eq!(patch_counts("GIT binary patch\nliteral 1\n"), (None, None));
}

#[test]
fn git_snapshot_and_lazy_diff_use_separate_layers() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let temp = TestDirectory::new("git-command");
    let repository = temp.path().join("repo");
    let home = temp.path().join("home");
    let config = temp.path().join("config");
    fs::create_dir_all(&repository).expect("create repository directory");
    fs::create_dir_all(&home).expect("create home directory");
    fs::create_dir_all(&config).expect("create config directory");
    run_git(&repository, &home, &config, &["init"]);
    run_git(
        &repository,
        &home,
        &config,
        &["config", "user.name", "Pi Test"],
    );
    run_git(
        &repository,
        &home,
        &config,
        &["config", "user.email", "pi@example.invalid"],
    );
    fs::write(repository.join("file.txt"), "base\n").expect("write base");
    run_git(&repository, &home, &config, &["add", "file.txt"]);
    run_git(&repository, &home, &config, &["commit", "-m", "base"]);
    fs::write(repository.join("file.txt"), "staged\n").expect("write staged");
    run_git(&repository, &home, &config, &["add", "file.txt"]);
    fs::write(repository.join("file.txt"), "staged\nworking\n").expect("write working");

    let options = RepositoryOptions {
        environment: isolated_environment(&home, &config),
        ..RepositoryOptions::default()
    };
    let backend =
        RepositoryBackend::discover_with_options(&repository, BackendPreference::Git, options)
            .expect("discover Git")
            .expect("Git repository");
    let snapshot = backend.snapshot().expect("capture Git snapshot");
    assert_eq!(
        backend.list_project_files().expect("list Git files"),
        ["file.txt"]
    );
    assert!(
        snapshot
            .changes
            .iter()
            .any(|row| row.layer == ChangeLayer::GitIndex)
    );
    assert!(
        snapshot
            .changes
            .iter()
            .any(|row| row.layer == ChangeLayer::GitWorkingTree)
    );
    let target = snapshot
        .changes
        .iter()
        .find(|row| row.layer == ChangeLayer::GitIndex)
        .expect("staged row")
        .target
        .clone();
    let diff = backend.load_diff(target).expect("load staged diff");
    assert!(diff.patch.contains("+staged"));
    assert_eq!(diff.additions, Some(1));
    assert_eq!(diff.deletions, Some(1));
    assert!(diff.exists);
}

#[test]
fn linked_git_worktree_is_an_independent_working_copy() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let temp = TestDirectory::new("git-worktree");
    let repository = temp.path().join("repo");
    let worktree = temp.path().join("worktree");
    let home = temp.path().join("home");
    let config = temp.path().join("config");
    fs::create_dir_all(&repository).expect("create repository directory");
    fs::create_dir_all(&home).expect("create home directory");
    fs::create_dir_all(&config).expect("create config directory");
    run_git(&repository, &home, &config, &["init"]);
    run_git(
        &repository,
        &home,
        &config,
        &["config", "user.name", "Pi Test"],
    );
    run_git(
        &repository,
        &home,
        &config,
        &["config", "user.email", "pi@example.invalid"],
    );
    fs::write(repository.join("file.txt"), "base\n").expect("write base");
    run_git(&repository, &home, &config, &["add", "file.txt"]);
    run_git(&repository, &home, &config, &["commit", "-m", "base"]);
    run_git(
        &repository,
        &home,
        &config,
        &[
            "worktree",
            "add",
            "-b",
            "linked",
            worktree.to_str().expect("UTF-8 worktree path"),
        ],
    );
    fs::write(worktree.join("file.txt"), "linked\n").expect("modify worktree");

    let options = RepositoryOptions {
        environment: isolated_environment(&home, &config),
        ..RepositoryOptions::default()
    };
    let backend =
        RepositoryBackend::discover_with_options(&worktree, BackendPreference::Git, options)
            .expect("discover linked worktree")
            .expect("Git worktree");
    assert_eq!(
        backend.location.workspace_root,
        worktree.canonicalize().expect("canonical worktree")
    );
    let snapshot = backend.snapshot().expect("worktree status");
    assert_eq!(snapshot.changes.len(), 1);
    assert_eq!(snapshot.changes[0].layer, ChangeLayer::GitWorkingTree);
}

#[test]
fn jj_snapshot_and_lazy_diff_use_the_current_change_only() {
    if Command::new("jj").arg("--version").output().is_err() {
        return;
    }
    let temp = TestDirectory::new("jj-command");
    let repository = temp.path().join("repo");
    let home = temp.path().join("home");
    let config = temp.path().join("config");
    fs::create_dir_all(&home).expect("create home directory");
    fs::create_dir_all(&config).expect("create config directory");
    run_jj(temp.path(), &home, &config, &["git", "init", "repo"]);
    fs::write(repository.join("a|b.txt"), "working\n").expect("write JJ file");

    let options = RepositoryOptions {
        environment: isolated_environment(&home, &config),
        ..RepositoryOptions::default()
    };
    let backend =
        RepositoryBackend::discover_with_options(&repository, BackendPreference::Jujutsu, options)
            .expect("discover JJ")
            .expect("JJ repository");
    let snapshot = backend.snapshot().expect("capture JJ snapshot");
    assert_eq!(
        backend.list_project_files().expect("list JJ files"),
        ["a|b.txt"]
    );
    assert!(matches!(snapshot.identity, SnapshotIdentity::Jujutsu(_)));
    assert_eq!(snapshot.changes.len(), 1);
    assert_eq!(snapshot.changes[0].layer, ChangeLayer::JujutsuWorkingCopy);
    let diff = backend
        .load_diff(snapshot.changes[0].target.clone())
        .expect("load JJ diff");
    assert!(diff.patch.contains("diff --git a/a|b.txt b/a|b.txt"));
}

fn run_git(repository: &Path, home: &Path, config: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", config)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("run Git command");
    assert!(
        output.status.success(),
        "Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_jj(repository: &Path, home: &Path, config: &Path, arguments: &[&str]) {
    let output = Command::new("jj")
        .args(arguments)
        .current_dir(repository)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", config)
        .output()
        .expect("run JJ command");
    assert!(
        output.status.success(),
        "JJ failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn isolated_environment(home: &Path, config: &Path) -> Vec<(OsString, OsString)> {
    vec![
        (OsString::from("HOME"), home.as_os_str().to_os_string()),
        (
            OsString::from("XDG_CONFIG_HOME"),
            config.as_os_str().to_os_string(),
        ),
        (OsString::from("GIT_CONFIG_NOSYSTEM"), OsString::from("1")),
        (OsString::from("GIT_TERMINAL_PROMPT"), OsString::from("0")),
    ]
}
