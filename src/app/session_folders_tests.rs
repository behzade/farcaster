use super::*;
use crate::app::infrastructure::persistence::StateStore;

fn folders() -> SessionFolders {
    SessionFolders {
        folders: vec![
            SessionFolder {
                id: 1,
                name: "Work".into(),
                ..Default::default()
            },
            SessionFolder {
                id: 2,
                name: "Personal".into(),
                ..Default::default()
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
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("state.sqlite");
    let mut folders = folders();
    folders.assign(42, Some(2));
    folders.set_color(2, Some(3));
    folders.set_project_color("/project".into(), Some(4));
    {
        let store = StateStore::open_at(&path).expect("open store");
        assert!(
            store
                .load_session_folders()
                .expect("load folders")
                .folders
                .is_empty()
        );
        store.save_session_folders(&folders).expect("save folders");
    }
    let restored = StateStore::open_at(&path)
        .expect("reopen store")
        .load_session_folders()
        .expect("restore folders");
    assert_eq!(restored.folders[1].name, "Personal");
    assert_eq!(restored.folder_for(42), Some(2));
    assert_eq!(restored.folders[1].color, Some(3));
    assert_eq!(
        restored.project_colors.get(&PathBuf::from("/project")),
        Some(&4)
    );
}

#[test]
fn creating_folder_from_drop_moves_only_the_dragged_session() {
    let mut folders = folders();
    folders.assign(10, Some(1));
    folders.assign(11, Some(1));
    folders.create("New folder".into(), Some(10));
    let created = folders.folders.last().expect("created folder");
    assert_eq!(created.name, "New folder");
    assert_eq!(folders.folder_for(10), Some(created.id));
    assert_eq!(folders.folder_for(11), Some(1));
    folders.create("Empty".into(), None);
    assert_eq!(folders.membership.len(), 2);
}

#[test]
fn new_folders_use_the_default_heading_color() {
    let mut folders = SessionFolders::default();
    folders.create("First".into(), None);
    folders.create("Second".into(), None);
    folders.remove(1);
    folders.create("Third".into(), None);
    assert_eq!(
        folders
            .folders
            .iter()
            .map(|folder| folder.color)
            .collect::<Vec<_>>(),
        [None, None]
    );
}

fn summary(
    id: &str,
    app_session_id: i64,
    project: &str,
    parent: Option<&str>,
    archived: bool,
) -> SessionSummary {
    SessionSummary::from_cached(
        id.into(),
        PathBuf::from(format!("/{id}.jsonl")),
        PathBuf::from(project),
        id.into(),
        String::new(),
        String::new(),
        parent.map(str::to_owned),
        std::time::SystemTime::UNIX_EPOCH,
        0,
        Default::default(),
        archived,
        false,
        String::new(),
    )
    .with_app_session_id(app_session_id)
}

fn draft(id: &str, app_session_id: i64, project: &str) -> DraftSession {
    let mut draft = DraftSession::with_id(
        Some(crate::agents::Backend::Pi),
        id.into(),
        PathBuf::from(project),
    );
    draft.app_session_id = app_session_id;
    draft
}

#[test]
fn folder_deletion_takes_its_chats_drafts_and_subagent_families() {
    let mut folders = SessionFolders::default();
    folders.create("Work".into(), None);
    let work_folder = folders.folders[0].id;
    for id in [10, 11, 12, 14, 20] {
        folders.assign(id, Some(work_folder));
    }

    let sessions = vec![
        summary("root", 10, "/work", None, false),
        summary("sub", 11, "/work", Some("root"), false),
        summary("archived", 12, "/work", None, true),
        summary("other", 13, "/other", None, false),
        summary("filed", 14, "/elsewhere", None, false),
    ];
    let drafts = vec![
        draft("work-draft", 20, "/work"),
        draft("other-draft", 21, "/other"),
    ];

    let deletion = folder_deletion(&sessions, &drafts, &folders, work_folder);

    assert_eq!(
        deletion.roots,
        vec![PathBuf::from("/root.jsonl"), PathBuf::from("/filed.jsonl")]
    );
    assert_eq!(deletion.drafts, vec!["work-draft".to_owned()]);
    let mut family_paths = deletion.family_paths.into_iter().collect::<Vec<_>>();
    family_paths.sort();
    assert_eq!(
        family_paths,
        vec![
            PathBuf::from("/filed.jsonl"),
            PathBuf::from("/root.jsonl"),
            PathBuf::from("/sub.jsonl"),
        ]
    );
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
