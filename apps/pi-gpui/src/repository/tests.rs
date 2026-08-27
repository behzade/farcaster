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

    let root = discover_available(temp.path(), BackendPreference::Auto)
        .expect("discover root")
        .expect("root repository");
    assert_eq!(root.location.kind, RepositoryKind::Jujutsu);

    let nested_backend = discover_available(&nested, BackendPreference::Auto)
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

    let error = discover_available(temp.path(), BackendPreference::Git)
        .expect_err("forced Git must not use JJ");
    assert!(matches!(
        error,
        RepositoryError::BackendUnavailable {
            kind: RepositoryKind::Git,
            ..
        }
    ));

    fs::create_dir(temp.path().join(".git")).expect("create Git marker");
    let git = discover_available(temp.path(), BackendPreference::Git)
        .expect("discover forced Git")
        .expect("Git repository");
    let jj = discover_available(temp.path(), BackendPreference::Jujutsu)
        .expect("discover forced JJ")
        .expect("JJ repository");
    assert_eq!(git.location.kind, RepositoryKind::Git);
    assert_eq!(jj.location.kind, RepositoryKind::Jujutsu);
}

#[test]
fn unavailable_backend_is_ignored_for_auto_and_stale_preference() {
    let temp = TestDirectory::new("unavailable");
    fs::create_dir(temp.path().join(".git")).expect("create Git marker");
    fs::create_dir(temp.path().join(".jj")).expect("create JJ marker");
    let missing = temp.path().join("missing");
    let options = RepositoryOptions {
        git_executable: std::env::current_exe()
            .expect("current executable")
            .into_os_string(),
        jj_executable: missing.into_os_string(),
        ..RepositoryOptions::default()
    };

    for preference in [BackendPreference::Auto, BackendPreference::Jujutsu] {
        let backend =
            RepositoryBackend::discover_with_options(temp.path(), preference, options.clone())
                .expect("ignore unavailable JJ")
                .expect("Git repository");
        assert_eq!(backend.location.kind, RepositoryKind::Git);
    }
}

#[test]
fn no_repository_is_distinct_from_failure() {
    let temp = TestDirectory::new("none");
    let result = RepositoryBackend::discover(temp.path(), BackendPreference::Auto)
        .expect("marker scan should succeed");
    assert!(result.is_none());
}

#[test]
fn jj_init_is_required_only_for_git_without_a_jj_marker() {
    let temp = TestDirectory::new("jj-init-required");
    fs::create_dir(temp.path().join(".git")).expect("create Git marker");
    let mut location = RepositoryLocation {
        kind: RepositoryKind::Git,
        workspace_root: temp.path().to_path_buf(),
        project_root: temp.path().to_path_buf(),
    };

    assert!(RepositoryBackend::jj_init_required(&location).expect("inspect Git repository"));
    fs::create_dir(temp.path().join(".jj")).expect("create JJ marker");
    assert!(!RepositoryBackend::jj_init_required(&location).expect("inspect colocated repository"));
    location.kind = RepositoryKind::Jujutsu;
    assert!(!RepositoryBackend::jj_init_required(&location).expect("inspect JJ repository"));
}

#[cfg(unix)]
#[test]
fn jj_init_runs_git_init_in_the_repository() {
    use std::os::unix::fs::PermissionsExt as _;

    let temp = TestDirectory::new("jj-init-command");
    let repository = temp.path().join("repo");
    let executable = temp.path().join("jj");
    let log = temp.path().join("jj.log");
    fs::create_dir_all(repository.join(".git")).expect("create Git repository");
    fs::write(
        &executable,
        "#!/bin/sh\nprintf '%s\\n' \"$PWD\" \"$@\" > \"$PI_TEST_LOG\"\n",
    )
    .expect("write fake JJ");
    let mut permissions = fs::metadata(&executable)
        .expect("read fake JJ metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions).expect("make fake JJ executable");
    let options = RepositoryOptions {
        jj_executable: executable.into_os_string(),
        environment: vec![(
            OsString::from("PI_TEST_LOG"),
            log.as_os_str().to_os_string(),
        )],
        ..RepositoryOptions::default()
    };

    RepositoryBackend::init_jj_colocated_with_options(&repository, options)
        .expect("initialize colocated JJ repository");

    let invocation = fs::read_to_string(log).expect("read fake JJ invocation");
    let expected_root = repository
        .canonicalize()
        .expect("canonical repository")
        .display()
        .to_string();
    assert_eq!(
        invocation.lines().collect::<Vec<_>>(),
        [expected_root.as_str(), "git", "init"]
    );
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
        backend
            .working_copy_totals(&snapshot)
            .expect("count Git working copy diff"),
        (Some(2), Some(1))
    );
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
        backend
            .working_copy_totals(&snapshot)
            .expect("count JJ working copy diff"),
        (Some(1), Some(0))
    );
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

#[test]
fn jj_identity_uses_closest_ancestor_bookmark() {
    if Command::new("jj").arg("--version").output().is_err() {
        return;
    }
    let temp = TestDirectory::new("jj-closest-bookmark");
    let repository = temp.path().join("repo");
    let home = temp.path().join("home");
    let config = temp.path().join("config");
    fs::create_dir_all(&home).expect("create home directory");
    fs::create_dir_all(&config).expect("create config directory");
    run_jj(temp.path(), &home, &config, &["git", "init", "repo"]);
    fs::write(repository.join("file.txt"), "base\n").expect("write JJ file");
    run_jj(&repository, &home, &config, &["commit", "-m", "base"]);
    run_jj(
        &repository,
        &home,
        &config,
        &["bookmark", "set", "main", "-r", "@-"],
    );

    let options = RepositoryOptions {
        environment: isolated_environment(&home, &config),
        ..RepositoryOptions::default()
    };
    let backend =
        RepositoryBackend::discover_with_options(&repository, BackendPreference::Jujutsu, options)
            .expect("discover JJ")
            .expect("JJ repository");
    let snapshot = backend.snapshot().expect("capture JJ snapshot");
    let SnapshotIdentity::Jujutsu(identity) = snapshot.identity else {
        panic!("expected Jujutsu identity");
    };
    assert!(identity.bookmarks.is_empty());
    assert_eq!(identity.closest_bookmarks, ["main"]);
    assert_eq!(identity.ahead, 1);
}

#[test]
fn git_detached_head_reports_nearest_local_branch() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let temp = TestDirectory::new("git-detached");
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
    run_git(&repository, &home, &config, &["branch", "-M", "main"]);
    run_git(&repository, &home, &config, &["checkout", "--detach"]);
    fs::write(repository.join("file.txt"), "detached\n").expect("write detached");
    run_git(&repository, &home, &config, &["add", "file.txt"]);
    run_git(&repository, &home, &config, &["commit", "-m", "detached"]);

    let options = RepositoryOptions {
        environment: isolated_environment(&home, &config),
        ..RepositoryOptions::default()
    };
    let backend =
        RepositoryBackend::discover_with_options(&repository, BackendPreference::Git, options)
            .expect("discover Git")
            .expect("Git repository");
    let snapshot = backend.snapshot().expect("capture Git snapshot");
    let SnapshotIdentity::Git(identity) = snapshot.identity else {
        panic!("expected Git identity");
    };
    assert!(identity.branch.is_none());
    assert_eq!(identity.nearest_branch.as_deref(), Some("main"));
    assert_eq!((identity.ahead, identity.behind), (1, 0));
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

fn discover_available(
    project: &Path,
    preference: BackendPreference,
) -> Result<Option<RepositoryBackend>, RepositoryError> {
    RepositoryBackend::discover_with_options(project, preference, available_options())
}

fn available_options() -> RepositoryOptions {
    let executable = std::env::current_exe()
        .expect("current executable")
        .into_os_string();
    RepositoryOptions {
        git_executable: executable.clone(),
        jj_executable: executable,
        ..RepositoryOptions::default()
    }
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
