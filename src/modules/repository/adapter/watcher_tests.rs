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
    assert!(targets.contains(&WatchTarget {
        path: project.clone(),
        mode: RecursiveMode::Recursive,
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
fn jujutsu_watches_only_project_files_and_ignores_its_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(workspace.join(".jj")).expect("jj metadata");
    let workspace = workspace.canonicalize().expect("workspace");

    let targets = watch_targets(&RepositoryLocation {
        kind: RepositoryKind::Jujutsu,
        workspace_root: workspace.clone(),
        project_root: workspace.clone(),
    })
    .expect("targets");

    assert_eq!(
        targets,
        [WatchTarget {
            path: workspace.clone(),
            mode: RecursiveMode::Recursive,
        }]
    );
    let metadata = workspace.join(".jj/repo/lock");
    let colocated_git_metadata = workspace.join(".git/refs/heads/main");
    let source = workspace.join("source.rs");
    assert_eq!(
        jujutsu_working_copy_event(Ok(
            Event::new(EventKind::Create(CreateKind::File)).add_path(metadata.clone())
        )),
        None
    );
    assert_eq!(
        jujutsu_working_copy_event(Ok(
            Event::new(EventKind::Create(CreateKind::File)).add_path(colocated_git_metadata)
        )),
        None
    );
    assert_eq!(
        jujutsu_working_copy_event(Ok(Event::new(EventKind::Create(CreateKind::File)))),
        None
    );
    assert_eq!(
        jujutsu_working_copy_event(Ok(
            Event::new(EventKind::Create(CreateKind::File)).add_path(source.clone())
        )),
        Some(RepositoryWatchEvent::Changed)
    );
    assert_eq!(
        jujutsu_working_copy_event(Ok(Event::new(EventKind::Create(CreateKind::File))
            .add_path(metadata)
            .add_path(source))),
        Some(RepositoryWatchEvent::Changed)
    );
}

#[test]
fn secondary_jujutsu_workspace_does_not_watch_shared_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let shared_repo = temp.path().join("primary/.jj/repo");
    fs::create_dir_all(workspace.join(".jj")).expect("workspace metadata");
    fs::create_dir_all(&shared_repo).expect("shared metadata");
    fs::write(workspace.join(".jj/repo"), "../../primary/.jj/repo").expect("repo pointer");
    let workspace = workspace.canonicalize().expect("workspace");

    let targets = watch_targets(&RepositoryLocation {
        kind: RepositoryKind::Jujutsu,
        workspace_root: workspace.clone(),
        project_root: workspace.clone(),
    })
    .expect("targets");

    assert_eq!(
        targets,
        [WatchTarget {
            path: workspace,
            mode: RecursiveMode::Recursive,
        }]
    );
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
