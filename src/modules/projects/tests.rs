use tempfile::tempdir;

use super::*;

#[test]
fn registry_round_trips_unique_existing_projects() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    fs::create_dir(&first)?;
    fs::create_dir(&second)?;
    let path = temp.path().join("state/projects.json");

    save_to(
        &path,
        &Registry {
            projects: vec![first.clone(), second.clone(), first.clone()],
            excluded_projects: Vec::new(),
            drafts: vec![DraftSession {
                id: "draft-one".into(),
                app_session_id: 7,
                project: first.clone(),
                created_ms: 1,
                submitted: true,
                session_path: Some(second.clone()),
                title: None,
            }],
        },
    )?;

    assert_eq!(
        load_from(&path)?,
        Registry {
            projects: vec![first.canonicalize()?, second.canonicalize()?],
            excluded_projects: Vec::new(),
            drafts: vec![DraftSession {
                id: "draft-one".into(),
                app_session_id: 7,
                project: first.canonicalize()?,
                created_ms: 1,
                submitted: true,
                session_path: Some(second.canonicalize()?),
                title: None,
            }],
        }
    );
    Ok(())
}

#[test]
fn registry_ignores_projects_that_no_longer_exist() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let path = temp.path().join("projects.json");
    save_to(
        &path,
        &Registry {
            projects: vec![temp.path().join("gone")],
            excluded_projects: Vec::new(),
            drafts: vec![DraftSession {
                id: "gone".into(),
                app_session_id: 8,
                project: temp.path().join("gone"),
                created_ms: 1,
                submitted: false,
                session_path: None,
                title: None,
            }],
        },
    )?;

    assert_eq!(load_from(&path)?, Registry::default());
    Ok(())
}

#[test]
fn linked_worktrees_are_distinct_projects_that_can_be_selected_and_restored()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let repository = temp.path().join("repository");
    let worktree = temp.path().join("worktree");
    let worktree_git_dir = repository.join(".git/worktrees/feature");
    fs::create_dir_all(&worktree_git_dir)?;
    fs::create_dir_all(&worktree)?;
    fs::write(worktree_git_dir.join("commondir"), "../..\n")?;
    fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", worktree_git_dir.display()),
    )?;
    fs::create_dir_all(repository.join(".git"))?;

    let repository = repository.canonicalize()?;
    let worktree = worktree.canonicalize()?;
    let mut projects = vec![repository.clone()];
    let mut excluded_projects = Vec::new();
    assert!(add_unique(&mut projects, worktree.clone()));
    assert!(!add_unique(&mut projects, worktree.clone()));
    assert_eq!(projects, vec![repository.clone(), worktree.clone()]);

    assert!(select(&mut projects, &excluded_projects, worktree.clone()));
    assert_eq!(projects, vec![worktree.clone(), repository.clone()]);
    assert!(remove(&mut projects, &mut excluded_projects, &worktree));
    assert!(restore(&mut excluded_projects, &worktree));
    assert!(add_visible(
        &mut projects,
        &excluded_projects,
        worktree.clone()
    ));
    assert_eq!(projects, vec![repository, worktree]);
    Ok(())
}

#[test]
fn selecting_a_project_moves_it_to_the_front_unless_it_was_removed() {
    let first = PathBuf::from("/first");
    let second = PathBuf::from("/second");
    let removed = PathBuf::from("/removed");
    let mut projects = vec![first.clone(), second.clone()];

    assert!(select(&mut projects, &[], second.clone()));
    assert_eq!(projects, vec![second.clone(), first]);
    assert!(!select(&mut projects, &[], second));
    assert!(!select(
        &mut projects,
        std::slice::from_ref(&removed),
        removed.clone()
    ));
}

#[test]
fn removing_a_project_only_changes_registered_matches() {
    let first = PathBuf::from("/first");
    let second = PathBuf::from("/second");
    let mut projects = vec![first.clone(), second.clone()];
    let mut excluded_projects = Vec::new();

    assert!(remove(&mut projects, &mut excluded_projects, &first));
    assert_eq!(projects, vec![second]);
    assert_eq!(excluded_projects, vec![first.clone()]);
    assert!(!remove(&mut projects, &mut excluded_projects, &first));
    assert!(restore(&mut excluded_projects, &first));
    assert!(excluded_projects.is_empty());
}

#[test]
fn only_unsubmitted_drafts_can_change_project() {
    let mut draft = DraftSession::new("draft".into(), 1, PathBuf::from("/first"), 1);
    assert!(draft.change_project(PathBuf::from("/second")));
    assert_eq!(draft.project, PathBuf::from("/second"));
    assert!(!draft.change_project(PathBuf::from("/second")));

    draft.submitted = true;
    assert!(!draft.change_project(PathBuf::from("/third")));
    assert_eq!(draft.project, PathBuf::from("/second"));
}

#[test]
fn old_registry_drafts_decode_as_unsubmitted() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempdir()?;
    let project = temp.path().join("project");
    fs::create_dir(&project)?;
    let path = temp.path().join("projects.json");
    fs::write(
        &path,
        serde_json::json!({
            "projects": [project.clone()],
            "drafts": [{"id": "legacy", "project": project, "created_ms": 3}]
        })
        .to_string(),
    )?;

    let registry = load_from(&path)?;
    assert_eq!(registry.drafts.len(), 1);
    assert!(!registry.drafts[0].submitted);
    assert_eq!(registry.drafts[0].session_path, None);
    Ok(())
}
