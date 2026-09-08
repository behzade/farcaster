use super::*;
use tempfile::tempdir;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn session(path: &Path, id: &str, cwd: &Path, parent: Option<&Path>, body: &str) -> TestResult {
    let mut file = File::create(path)?;
    writeln!(
        file,
        "{}",
        serde_json::json!({"type":"session","version":3,"id":id,"cwd":cwd,"parentSession":parent,"unknown":"kept"})
    )?;
    write!(file, "{body}")?;
    Ok(())
}

#[test]
fn moves_family_and_preserves_ids_history_and_relationships() -> TestResult {
    let temp = tempdir()?;
    let source_project = temp.path().join("source-project");
    let target_project = temp.path().join("target-project");
    let source_dir = temp.path().join("source-sessions");
    let target_dir = temp.path().join("target-sessions");
    fs::create_dir_all(&source_project)?;
    fs::create_dir_all(&target_project)?;
    fs::create_dir_all(&source_dir)?;
    let root = source_dir.join("root.jsonl");
    let child = source_dir.join("child.jsonl");
    let grandchild = source_dir.join("grandchild.jsonl");
    session(&root, "root", &source_project, None, "root body\n")?;
    session(
        &child,
        "child",
        &source_project,
        Some(&root),
        "child body\n",
    )?;
    session(
        &grandchild,
        "grandchild",
        &source_project,
        Some(&child),
        "grandchild body\n",
    )?;
    let members = vec![
        TransferMember {
            path: root.clone(),
            id: "root".into(),
            parent_id: None,
        },
        TransferMember {
            path: child.clone(),
            id: "child".into(),
            parent_id: Some("root".into()),
        },
        TransferMember {
            path: grandchild.clone(),
            id: "grandchild".into(),
            parent_id: Some("child".into()),
        },
    ];

    let moved = move_family(&members, "root", &target_project, &target_dir)?;

    assert_eq!(moved.root, target_dir.join("root.jsonl"));
    assert!(!root.exists() && !child.exists() && !grandchild.exists());
    let root_text = fs::read_to_string(target_dir.join("root.jsonl"))?;
    let child_text = fs::read_to_string(target_dir.join("child.jsonl"))?;
    let grandchild_text = fs::read_to_string(target_dir.join("grandchild.jsonl"))?;
    assert!(root_text.contains("\"id\":\"root\""));
    assert!(root_text.contains("\"unknown\":\"kept\""));
    assert!(
        !root_text
            .lines()
            .next()
            .unwrap_or_default()
            .contains("parentSession")
    );
    assert!(root_text.ends_with("root body\n"));
    assert!(child_text.contains(&target_dir.join("root.jsonl").to_string_lossy().into_owned()));
    assert!(
        grandchild_text.contains(
            &target_dir
                .join("child.jsonl")
                .to_string_lossy()
                .into_owned()
        )
    );
    Ok(())
}

#[test]
fn in_place_move_rewrites_header_without_creating_a_duplicate() -> TestResult {
    let temp = tempdir()?;
    let source_project = temp.path().join("source-project");
    let target_project = temp.path().join("target-project");
    fs::create_dir(&source_project)?;
    fs::create_dir(&target_project)?;
    let root = temp.path().join("root.jsonl");
    session(&root, "root", &source_project, None, "body\n")?;
    let members = [TransferMember {
        path: root.clone(),
        id: "root".into(),
        parent_id: None,
    }];

    let moved = move_family(&members, "root", &target_project, temp.path())?;

    assert_eq!(moved.root, root);
    let text = fs::read_to_string(&root)?;
    assert!(text.contains(&target_project.to_string_lossy().into_owned()));
    assert!(text.ends_with("body\n"));
    assert_eq!(
        fs::read_dir(temp.path())?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "jsonl"))
            .count(),
        1
    );
    Ok(())
}

#[test]
fn destination_collision_leaves_source_untouched() -> TestResult {
    let temp = tempdir()?;
    let source_project = temp.path().join("source-project");
    let target_project = temp.path().join("target-project");
    let target_dir = temp.path().join("target-sessions");
    fs::create_dir(&source_project)?;
    fs::create_dir(&target_project)?;
    fs::create_dir(&target_dir)?;
    let root = temp.path().join("root.jsonl");
    session(&root, "root", &source_project, None, "source\n")?;
    fs::write(target_dir.join("root.jsonl"), "existing\n")?;
    let members = [TransferMember {
        path: root.clone(),
        id: "root".into(),
        parent_id: None,
    }];

    assert!(move_family(&members, "root", &target_project, &target_dir).is_err());
    assert!(root.exists());
    assert_eq!(
        fs::read_to_string(target_dir.join("root.jsonl"))?,
        "existing\n"
    );
    Ok(())
}

#[test]
fn malformed_member_leaves_every_source_untouched() -> TestResult {
    let temp = tempdir()?;
    let project = temp.path().join("project");
    let target = temp.path().join("target");
    fs::create_dir(&project)?;
    let root = temp.path().join("root.jsonl");
    let child = temp.path().join("child.jsonl");
    session(&root, "root", &project, None, "root\n")?;
    fs::write(&child, "not json\n")?;
    let members = vec![
        TransferMember {
            path: root.clone(),
            id: "root".into(),
            parent_id: None,
        },
        TransferMember {
            path: child.clone(),
            id: "child".into(),
            parent_id: Some("root".into()),
        },
    ];

    assert!(move_family(&members, "root", &project, &target).is_err());
    assert!(root.exists() && child.exists());
    assert!(!target.join("root.jsonl").exists());
    Ok(())
}
