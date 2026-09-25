use std::{collections::BTreeSet, path::Path, path::PathBuf};

use super::{project_color, project_rows};
use crate::{
    agents::Backend,
    app::views::session_rail::{
        folders::FolderRow,
        groups::{ActiveSessionItem, SessionRailItem, SessionRailKind},
    },
    sessions::{DraftSession, SessionSummary, UsageSummary},
};

fn session(id: &str, app_session_id: i64, project: &Path, archived: bool) -> SessionSummary {
    SessionSummary::from_cached(
        id.into(),
        PathBuf::from(format!("/{id}.jsonl")),
        project.to_path_buf(),
        id.into(),
        String::new(),
        String::new(),
        None,
        std::time::SystemTime::UNIX_EPOCH,
        0,
        UsageSummary::default(),
        archived,
        false,
        String::new(),
    )
    .with_app_session_id(app_session_id)
}

fn rail_session(id: &str, app_session_id: i64, project: &Path) -> ActiveSessionItem {
    ActiveSessionItem::Session(SessionRailItem {
        session: session(id, app_session_id, project, false),
        kind: SessionRailKind::Project,
    })
}

fn draft(id: i64, project: &Path) -> ActiveSessionItem {
    let mut draft = DraftSession::with_id(
        Some(Backend::Pi),
        format!("draft-{id}"),
        project.to_path_buf(),
    );
    draft.app_session_id = id;
    ActiveSessionItem::Draft(draft)
}

fn row_ids(rows: &[FolderRow]) -> Vec<String> {
    rows.iter()
        .filter_map(|row| match row {
            FolderRow::Session(item) => Some(item.app_session_id().to_string()),
            FolderRow::Project(project, _) => Some(project.display().to_string()),
            FolderRow::Header(id, name) => Some(format!("{id}:{name}")),
            FolderRow::New => None,
        })
        .collect()
}

#[test]
fn project_rows_group_sessions_under_one_header_per_project() {
    let alpha = PathBuf::from("/alpha");
    let beta = PathBuf::from("/beta");
    let rows = project_rows(
        vec![
            rail_session("a1", 1, &alpha),
            rail_session("b1", 2, &beta),
            rail_session("a2", 3, &alpha),
        ],
        &BTreeSet::new(),
    );

    assert_eq!(row_ids(&rows), ["/alpha", "1", "3", "/beta", "2"]);
}

#[test]
fn project_rows_keep_drafts_above_every_project() {
    let alpha = PathBuf::from("/alpha");
    let rows = project_rows(
        vec![rail_session("a1", 1, &alpha), draft(9, &alpha)],
        &BTreeSet::new(),
    );

    assert_eq!(row_ids(&rows), ["9", "/alpha", "1"]);
}

#[test]
fn a_collapsed_project_hides_its_sessions() {
    let alpha = PathBuf::from("/alpha");
    let beta = PathBuf::from("/beta");
    let collapsed = BTreeSet::from([alpha.clone()]);
    let rows = project_rows(
        vec![rail_session("a1", 1, &alpha), rail_session("b1", 2, &beta)],
        &collapsed,
    );

    assert_eq!(row_ids(&rows), ["/alpha", "/beta", "2"]);
    assert!(matches!(&rows[0], FolderRow::Project(project, true) if *project == alpha));
}

#[test]
fn project_colors_are_stable_and_spread_across_the_palette() {
    let alpha = PathBuf::from("/alpha");
    assert_eq!(project_color(&alpha), project_color(&alpha.clone()));

    let mut distinct = Vec::new();
    for index in 0..8 {
        let color = project_color(Path::new(&format!("/workspace/project-{index}")));
        if !distinct.contains(&color) {
            distinct.push(color);
        }
    }
    assert!(
        distinct.len() >= 3,
        "eight projects only shared {} colors",
        distinct.len()
    );
}
