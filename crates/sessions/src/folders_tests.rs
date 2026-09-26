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
