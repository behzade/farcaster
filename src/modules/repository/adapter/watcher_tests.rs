use notify::{
    Event,
    event::{AccessKind, CreateKind},
};

use super::*;

#[test]
fn ignores_access_and_reports_changes() {
    assert_eq!(
        repository_event(Ok(Event::new(EventKind::Access(AccessKind::Any)))),
        None
    );
    assert_eq!(
        repository_event(Ok(Event::new(EventKind::Create(CreateKind::File)))),
        Some(RepositoryWatchEvent::Changed)
    );
}

#[test]
fn discovery_watches_project_and_ancestors_but_only_accepts_repository_markers() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join("parent/project");
    fs::create_dir_all(&project).expect("project");
    let project = project.canonicalize().expect("project");
    let parent = project.parent().expect("parent").to_path_buf();

    let targets = discovery_targets(&project).expect("targets");
    assert!(
        targets
            .iter()
            .all(|target| target.mode == RecursiveMode::NonRecursive)
    );
    assert!(targets.contains(&WatchTarget {
        path: project.clone(),
        mode: RecursiveMode::NonRecursive,
    }));
    assert!(targets.contains(&WatchTarget {
        path: parent,
        mode: RecursiveMode::NonRecursive,
    }));
    assert_eq!(
        discovery_event(Ok(
            Event::new(EventKind::Create(CreateKind::Folder)).add_path(project.join(".jj"))
        )),
        Some(RepositoryWatchEvent::Changed)
    );
    assert_eq!(
        discovery_event(Ok(
            Event::new(EventKind::Create(CreateKind::File)).add_path(project.join("file.rs"))
        )),
        None
    );
}

#[test]
fn nested_git_project_watches_project_and_repository_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let project = workspace.join("app");
    fs::create_dir_all(workspace.join(".git")).expect("git metadata");
    fs::create_dir_all(&project).expect("project");
    let workspace = workspace.canonicalize().expect("workspace");
    let project = project.canonicalize().expect("project");

    let targets = watch_targets(&RepositoryLocation {
        kind: RepositoryKind::Git,
        workspace_root: workspace.clone(),
        project_root: project.clone(),
    })
    .expect("targets");

    assert!(targets.contains(&WatchTarget {
        path: project,
        mode: RecursiveMode::Recursive,
    }));
    assert!(targets.contains(&WatchTarget {
        path: workspace.join(".git"),
        mode: RecursiveMode::Recursive,
    }));
}

#[test]
fn jujutsu_refreshes_for_commits_but_not_snapshot_bookkeeping() {
    let metadata = JujutsuMetadata {
        repo: PathBuf::from("/workspace/.jj/repo"),
        git: vec![WatchTarget {
            path: PathBuf::from("/workspace/.git"),
            mode: RecursiveMode::Recursive,
        }],
    };
    for path in [
        "/workspace/source.rs",
        "/workspace/.jj/repo/op_heads/heads/abcdef0123",
        "/workspace/.git/HEAD",
        "/workspace/.git/refs/heads/main",
        "/workspace/.git/packed-refs",
    ] {
        assert_eq!(
            metadata.classify(Ok(
                Event::new(EventKind::Create(CreateKind::File)).add_path(path.into())
            )),
            Some(RepositoryWatchEvent::Changed),
            "{path}"
        );
    }
    for path in [
        "/workspace/.jj/working_copy/tree_state",
        "/workspace/.jj/working_copy/lock",
        "/workspace/.jj/repo/op_heads/lock",
        "/workspace/.jj/repo/op_heads/heads/.tmp123",
        "/workspace/.git/HEAD.lock",
        "/workspace/.git/refs/heads/main.lock",
        "/workspace/.git/index",
        "/workspace/.git/index.lock",
    ] {
        assert_eq!(
            metadata.classify(Ok(
                Event::new(EventKind::Create(CreateKind::File)).add_path(path.into())
            )),
            None,
            "{path}"
        );
    }
    assert_eq!(
        metadata
            .classify(Ok(Event::new(EventKind::Access(AccessKind::Any))
                .add_path("/workspace/.git/HEAD".into()))),
        None
    );
}

#[test]
fn nested_jujutsu_project_observes_shared_operations_and_git_commits() {
    use std::time::{Duration, Instant};

    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let project = workspace.join("app");
    // The shared repository need not have a .jj component in its path.
    let shared_repo = temp.path().join("shared-repo");
    fs::create_dir_all(shared_repo.join("op_heads/heads")).expect("test operation should succeed");
    fs::create_dir_all(workspace.join(".jj")).expect("test operation should succeed");
    fs::create_dir_all(workspace.join(".git/refs/heads")).expect("test operation should succeed");
    fs::create_dir_all(&project).expect("test operation should succeed");
    fs::write(workspace.join(".jj/repo"), "../../shared-repo")
        .expect("test operation should succeed");
    let location = RepositoryLocation {
        kind: RepositoryKind::Jujutsu,
        workspace_root: workspace
            .canonicalize()
            .expect("test operation should succeed"),
        project_root: project
            .canonicalize()
            .expect("test operation should succeed"),
    };
    let metadata = JujutsuMetadata::resolve(&location).expect("test operation should succeed");
    assert!(!metadata.changed(&shared_repo.join("op_heads/lock")));
    let (_watcher, events) =
        RepositoryWatcher::start(&location).expect("test operation should succeed");
    for path in [
        shared_repo.join("op_heads/heads/abcdef0123"),
        workspace.join(".git/HEAD"),
        workspace.join(".git/refs/heads/main"),
    ] {
        while events.try_recv().is_ok() {}
        fs::write(&path, "commit").expect("test operation should succeed");
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Ok(event) = events.try_recv() {
                assert_eq!(event, RepositoryWatchEvent::Changed);
                break;
            }
            assert!(Instant::now() < deadline, "missed {}", path.display());
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[test]
fn linked_worktree_watches_common_metadata_that_contains_private_state() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("worktree");
    let git_dir = temp.path().join("main/.git/worktrees/worktree");
    let common_dir = temp.path().join("main/.git");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(&git_dir).expect("worktree metadata");
    fs::write(
        workspace.join(".git"),
        format!("gitdir: {}\n", git_dir.display()),
    )
    .expect("git pointer");
    fs::write(git_dir.join("commondir"), "../..\n").expect("common pointer");
    let workspace = workspace.canonicalize().expect("workspace");
    let git_dir = git_dir.canonicalize().expect("worktree metadata");
    let common_dir = common_dir.canonicalize().expect("common metadata");

    let targets = watch_targets(&RepositoryLocation {
        kind: RepositoryKind::Git,
        workspace_root: workspace.clone(),
        project_root: workspace,
    })
    .expect("targets");

    assert!(
        targets
            .iter()
            .any(|target| { target.path == common_dir && target.mode == RecursiveMode::Recursive })
    );
    assert!(git_dir.starts_with(&common_dir));
}
