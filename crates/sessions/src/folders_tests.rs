use super::*;

#[test]
fn separating_project_groups_preserves_custom_folders_and_explicit_assignments() {
    let mut folders = SessionFolders {
        folders: vec![
            SessionFolder {
                id: 1,
                name: "Later".into(),
                collapsed: true,
                ..Default::default()
            },
            SessionFolder {
                id: 2,
                name: "Automatic".into(),
                project: Some("/auto".into()),
                ..Default::default()
            },
            SessionFolder {
                id: 3,
                name: "Filed project".into(),
                project: Some("/filed".into()),
                ..Default::default()
            },
        ],
        membership: BTreeMap::from([(10, 1), (20, 3)]),
        session_colors: BTreeMap::from([(10, 4)]),
        ..Default::default()
    };
    folders.separate_project_groups();
    folders.separate_project_groups();
    assert_eq!(
        folders
            .folders
            .iter()
            .map(|folder| folder.id)
            .collect::<Vec<_>>(),
        [1, 3]
    );
    assert!(folders.folders[0].collapsed);
    assert!(
        folders
            .folders
            .iter()
            .all(|folder| folder.project.is_none())
    );
    assert_eq!(folders.folder_for(10), Some(1));
    assert_eq!(folders.folder_for(20), Some(3));
    assert_eq!(folders.session_color(10), Some(4));
    folders.create("New".into(), Some(30));
    assert_eq!(folders.folder_for(30), Some(4));
}

#[test]
fn group_colors_support_default_and_preserve_legacy_values() {
    let mut folders: SessionFolders = serde_json::from_str(
        r#"{"folders":[{"id":1,"name":"Saved","color":3}],"membership":{"7":1}}"#,
    )
    .expect("legacy folders");
    assert_eq!(folders.folders[0].color, Some(3));
    assert!(folders.set_color(1, None));
    folders.create("New".into(), None);
    assert!(folders.folders.iter().all(|folder| folder.color.is_none()));
    assert!(folders.set_color(1, Some(4)));
    assert!(folders.set_project_color("/project".into(), Some(2)));
    let mut restored: SessionFolders =
        serde_json::from_str(&serde_json::to_string(&folders).expect("encode folders"))
            .expect("decode folders");
    assert_eq!(restored.folders[0].color, Some(4));
    assert_eq!(
        restored.project_colors.get(&PathBuf::from("/project")),
        Some(&2)
    );
    assert_eq!(restored.folder_for(7), Some(1));
    assert!(restored.set_project_color("/project".into(), None));
    assert!(restored.project_colors.is_empty());
}

#[test]
fn group_orders_are_independent_and_keep_hidden_groups() {
    let mut folders = SessionFolders::default();
    for name in ["First", "Hidden", "Last"] {
        folders.create(name.into(), None);
    }
    folders.assign(7, Some(1));
    folders.set_collapsed(1, true);
    folders.set_color(1, Some(3));
    assert!(folders.remember_projects(["/a", "/hidden", "/z"].map(PathBuf::from)));
    assert!(folders.reorder_folder(3, 1, false));
    assert_eq!(
        folders.folders.iter().map(|f| f.id).collect::<Vec<_>>(),
        [3, 1, 2]
    );
    assert!(folders.reorder_project(
        std::path::Path::new("/z"),
        std::path::Path::new("/a"),
        false
    ));
    assert_eq!(
        folders.project_order,
        ["/z", "/a", "/hidden"].map(PathBuf::from)
    );
    assert!(!folders.reorder_folder(3, 1, false));
    assert!(!folders.reorder_folder(99, 1, false));
    assert!(!folders.reorder_folder(1, 1, true));
    assert!(folders.reorder_folder(3, 2, true));
    assert_eq!(
        folders.folders.iter().map(|f| f.id).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(folders.folder_for(7), Some(1));
    assert!(folders.folders[0].collapsed);
    assert_eq!(folders.folders[0].color, Some(3));
    assert!(folders.remember_projects(["/new", "/a", "/new"].map(PathBuf::from)));
    assert_eq!(
        folders.project_order,
        ["/z", "/a", "/hidden", "/new"].map(PathBuf::from)
    );
    assert!(!folders.remember_projects(["/a"].map(PathBuf::from)));
}
