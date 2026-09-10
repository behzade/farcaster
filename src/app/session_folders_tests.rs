use super::*;

fn folders() -> SessionFolders {
    SessionFolders {
        folders: vec![
            SessionFolder {
                id: 1,
                name: "Work".into(),
            },
            SessionFolder {
                id: 2,
                name: "Personal".into(),
            },
        ],
        ..Default::default()
    }
}

#[test]
fn session_folders_move_and_delete_without_losing_other_assignments() {
    let mut folders = folders();
    folders.assign(10, Some(1));
    folders.assign(11, Some(1));
    folders.assign(10, Some(2));
    assert_eq!(folders.folder_for(10), Some(2));
    folders.remove(1);
    assert_eq!(folders.folder_for(11), None);
    assert_eq!(folders.folder_for(10), Some(2));
    folders.assign(10, None);
    assert_eq!(folders.folder_for(10), None);
}

#[test]
fn session_folders_ignore_invalid_ids() {
    let mut folders = folders();
    folders.assign(0, Some(1));
    folders.assign(1, Some(99));
    assert!(folders.membership.is_empty());
    folders.membership.insert(2, 99);
    assert_eq!(folders.folder_for(2), None);
}

#[test]
fn session_folders_survive_database_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.sqlite");
    let mut folders = folders();
    folders.assign(42, Some(2));
    {
        let store = StateStore::open_at(&path).unwrap();
        assert!(store.load_session_folders().unwrap().folders.is_empty());
        store.save_session_folders(&folders).unwrap();
    }
    let restored = StateStore::open_at(&path)
        .unwrap()
        .load_session_folders()
        .unwrap();
    assert_eq!(restored.folders[1].name, "Personal");
    assert_eq!(restored.folder_for(42), Some(2));
}

#[test]
fn creating_folder_from_drop_moves_only_the_dragged_session() {
    let mut folders = folders();
    folders.assign(10, Some(1));
    folders.assign(11, Some(1));
    folders.create("New folder".into(), Some(10));
    let created = folders.folders.last().unwrap();
    assert_eq!(created.name, "New folder");
    assert_eq!(folders.folder_for(10), Some(created.id));
    assert_eq!(folders.folder_for(11), Some(1));
    folders.create("Empty".into(), None);
    assert_eq!(folders.membership.len(), 2);
}

#[test]
fn move_menu_marks_archive_as_current_without_losing_saved_folder() {
    let mut folders = folders();
    folders.assign(10, Some(1));
    assert_eq!(folders.destination(10, false), FolderDestination::Folder(1));
    assert_eq!(folders.destination(10, true), FolderDestination::Archived);
    assert_eq!(folders.folder_for(10), Some(1));
    folders.assign(10, None);
    assert_eq!(folders.destination(10, false), FolderDestination::Active);
}
