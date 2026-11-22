use crate::agents::Backend;
use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};

use super::*;
use crate::sessions::UsageSummary;

thread_local! {
    static LIST_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn record_list_build() {
    LIST_BUILDS.set(LIST_BUILDS.get() + 1);
}

#[gpui::test]
fn sidebar_prepares_session_lists_once_for_both_panels(cx: &mut gpui::TestAppContext) {
    use gpui::{IntoElement as _, ListAlignment, ListState, px};
    use std::cell::RefCell;

    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::sidebar_prepares_session_lists_once_for_both_panels"
        ),
        cx,
        |cx, app, _, project| {
            cx.update(|_, cx| {
                app.update(cx, |app, _| {
                    app.sessions.visible = vec![
                        session("active", 1, project, false),
                        session("archived", 2, project, true),
                    ]
                    .into();
                    app.sessions.drafts.clear();
                });
                for collapsed in [true, false] {
                    app.update(cx, |app, _| {
                        app.sessions.archived_expanded = !collapsed;
                    });
                    LIST_BUILDS.set(0);
                    let list = ListState::new(0, ListAlignment::Top, px(0.));
                    let prepared = app.read(cx).prepare_active_rows(&list, &RefCell::default());
                    let _sidebar = app
                        .read(cx)
                        .render_sessions(app.downgrade(), false, false, list, prepared)
                        .into_any_element();
                    assert_eq!(
                        LIST_BUILDS.get(),
                        1,
                        "panel sizing must reuse prepared counts"
                    );
                }
            });
        },
    );
}

#[test]
fn active_sidebar_counts_archives_without_preparing_their_rows() {
    let alpha = PathBuf::from("/alpha");
    let beta = PathBuf::from("/beta");
    let sessions = vec![
        session("active-one", 1, &alpha, false),
        session("active-two", 2, &alpha, false),
        session("shadowed", 3, &alpha, true),
        session("archived", 4, &alpha, true),
        session("other-project", 5, &beta, true),
    ];
    let mut draft = DraftSession::with_id(Some(Backend::Pi), "shadow".into(), alpha.clone());
    draft.app_session_id = 3;
    draft.submitted = true;
    assert!(draft.set_archived(true));
    let drafts = [draft];
    let ids = |items: &[ActiveSessionItem]| {
        items
            .iter()
            .map(ActiveSessionItem::app_session_id)
            .collect::<Vec<_>>()
    };
    for filter in [None, Some(alpha.as_path()), Some(beta.as_path())] {
        let full = session_rail_lists_for_roots(sessions.iter(), &drafts, filter, &[1, 2]);
        let active =
            active_session_rail_lists_for_roots(sessions.iter(), &drafts, filter, &[1, 2], |_| {});
        let archived = archived_session_rows_for_roots(sessions.iter(), &drafts, filter);
        assert_eq!(ids(&archived), ids(&full.archived));
        assert_eq!(active.archived_count, full.archived.len());
        assert!(
            active.archived.is_empty(),
            "the active sidebar only needs an archive count"
        );
        assert_eq!(ids(&active.active), ids(&full.active));
    }
}

fn archived_session(item: &ActiveSessionItem) -> &SessionRailItem {
    match item {
        ActiveSessionItem::Session(item) => item,
        ActiveSessionItem::Draft(_) => panic!("expected an archived chat, found a draft"),
    }
}

#[test]
fn unsubmitted_drafts_stay_above_newer_sessions() {
    let alpha = PathBuf::from("/alpha");
    let beta = PathBuf::from("/beta");
    let mut draft = DraftSession::with_id(Some(Backend::Pi), "draft".into(), alpha.clone());
    draft.app_session_id = 2;
    let sessions = vec![
        session("old", 1, &alpha, false),
        session("new", 3, &beta, false),
    ];

    let lists = session_rail_lists(&sessions, &[draft], None, &[]);

    assert_eq!(
        lists
            .active
            .iter()
            .map(ActiveSessionItem::app_session_id)
            .collect::<Vec<_>>(),
        [2, 3, 1]
    );
}

#[test]
fn submitted_chats_can_be_archived_before_their_session_record_arrives() {
    let project = PathBuf::from("/project");
    let mut draft = DraftSession::with_id(Some(Backend::Pi), "draft".into(), project.clone());
    draft.app_session_id = 2;
    let mut archived = DraftSession::with_id(Some(Backend::Pi), "old".into(), project.clone());
    archived.app_session_id = 1;
    archived.submitted = true;
    assert!(archived.set_archived(true));

    let lists = session_rail_lists(&[], &[draft, archived], None, &[]);

    assert_eq!(
        lists
            .active
            .iter()
            .map(ActiveSessionItem::app_session_id)
            .collect::<Vec<_>>(),
        [2]
    );
    assert_eq!(
        lists
            .archived
            .iter()
            .map(ActiveSessionItem::app_session_id)
            .collect::<Vec<_>>(),
        [1]
    );
}

#[test]
fn an_archived_chat_shadows_the_session_it_writes_into() {
    let project = PathBuf::from("/project");
    let mut draft = DraftSession::with_id(Some(Backend::Pi), "draft".into(), project.clone());
    draft.app_session_id = 7;
    draft.submitted = true;
    assert!(draft.set_archived(true));
    let mut session = session("live", 7, &project, true);
    session.archived = true;

    let lists = session_rail_lists(&[session], &[draft], None, &[]);

    assert!(lists.active.is_empty());
    assert_eq!(lists.archived.len(), 1);
    assert!(matches!(lists.archived[0], ActiveSessionItem::Draft(_)));
}

#[test]
fn draft_priority_preserves_manual_order_within_drafts_and_chats() {
    let project = PathBuf::from("/project");
    let mut draft = DraftSession::with_id(Some(Backend::Pi), "draft".into(), project.clone());
    draft.app_session_id = 2;
    let mut other_draft = draft.clone();
    other_draft.id = "other".into();
    other_draft.app_session_id = 4;
    let mut submitted = draft.clone();
    submitted.id = "submitted".into();
    submitted.app_session_id = 3;
    submitted.submitted = true;
    let sessions = vec![session("session", 1, &project, false)];

    let lists = session_rail_lists(
        &sessions,
        &[draft, other_draft, submitted],
        None,
        &[1, 2, 3, 4],
    );

    assert_eq!(
        lists
            .active
            .iter()
            .map(ActiveSessionItem::app_session_id)
            .collect::<Vec<_>>(),
        [2, 4, 1, 3]
    );
}

#[test]
fn manual_order_overrides_id_order_and_new_ids_stay_first() {
    let project = PathBuf::from("/project");
    let sessions = vec![
        session("new", 4, &project, false),
        session("three", 3, &project, false),
        session("two", 2, &project, false),
        session("one", 1, &project, false),
    ];

    let lists = session_rail_lists(&sessions, &[], None, &[1, 3, 2]);

    assert_eq!(
        lists
            .active
            .iter()
            .map(ActiveSessionItem::app_session_id)
            .collect::<Vec<_>>(),
        [4, 1, 3, 2]
    );
}

#[test]
fn reorder_uses_before_and_after_insertion_gaps() {
    assert_eq!(
        reordered_session_ids(&[4, 3, 2, 1], 4, 2, ReorderPosition::After),
        Some(vec![3, 2, 4, 1])
    );
    assert_eq!(
        reordered_session_ids(&[4, 3, 2, 1], 4, 1, ReorderPosition::After),
        Some(vec![3, 2, 1, 4])
    );
    assert_eq!(
        reordered_session_ids(&[4, 3, 2, 1], 1, 3, ReorderPosition::Before),
        Some(vec![4, 1, 3, 2])
    );
    assert_eq!(
        reordered_session_ids(&[4, 3, 2, 1], 3, 3, ReorderPosition::Before),
        None
    );
}

#[test]
fn filtered_reorder_preserves_hidden_row_positions() {
    assert_eq!(
        merge_visible_session_order(&[5, 4, 3, 2, 1], &[1, 3, 5]),
        [1, 4, 3, 2, 5]
    );
}

#[test]
fn project_filter_keeps_a_flat_subset() {
    let alpha = PathBuf::from("/alpha");
    let beta = PathBuf::from("/beta");
    let mut alpha_draft =
        DraftSession::with_id(Some(Backend::Pi), "alpha-draft".into(), alpha.clone());
    alpha_draft.app_session_id = 4;
    let mut beta_draft =
        DraftSession::with_id(Some(Backend::Pi), "beta-draft".into(), beta.clone());
    beta_draft.app_session_id = 3;
    let sessions = vec![
        session("alpha-active", 2, &alpha, false),
        session("beta-archived", 1, &beta, true),
    ];

    let lists = session_rail_lists(
        &sessions,
        &[alpha_draft, beta_draft],
        Some(beta.as_path()),
        &[],
    );

    assert_eq!(lists.active.len(), 1);
    assert_eq!(lists.active[0].app_session_id(), 3);
    assert_eq!(lists.archived.len(), 1);
    assert_eq!(lists.archived[0].app_session_id(), 1);
}

#[test]
fn archived_sessions_are_sorted_by_recency_not_imported_id() {
    let project = PathBuf::from("/project");
    let mut recent = session("recent", 1, &project, true);
    recent.modified = SystemTime::UNIX_EPOCH + Duration::from_secs(2);
    let mut old = session("old", 2, &project, true);
    old.modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1);

    let lists = session_rail_lists(&[old, recent], &[], None, &[]);

    assert_eq!(
        lists
            .archived
            .iter()
            .map(|item| archived_session(item).session.id.as_str())
            .collect::<Vec<_>>(),
        ["recent", "old"]
    );
}

#[test]
fn promotion_identity_is_rendered_once_and_prefers_the_draft() {
    let project = PathBuf::from("/project");
    let mut draft = DraftSession::with_id(Some(Backend::Pi), "draft".into(), project.clone());
    draft.app_session_id = 7;
    draft.submitted = true;
    let persisted = session("persisted", 7, &project, false);

    let lists = session_rail_lists(&[persisted], &[draft], None, &[]);

    assert_eq!(lists.active.len(), 1);
    assert!(matches!(lists.active[0], ActiveSessionItem::Draft(_)));
}

#[test]
fn unassigned_fallback_sessions_are_not_deduplicated() {
    let project = PathBuf::from("/project");
    let sessions = vec![
        session("one", 0, &project, false),
        session("two", 0, &project, false),
    ];

    let lists = session_rail_lists(&sessions, &[], None, &[]);

    assert_eq!(lists.active.len(), 2);
}

fn session(id: &str, app_session_id: i64, project: &Path, archived: bool) -> SessionSummary {
    SessionSummary::from_cached(
        id.into(),
        PathBuf::from(format!("/{id}.jsonl")),
        project.to_path_buf(),
        id.into(),
        String::new(),
        String::new(),
        None,
        SystemTime::UNIX_EPOCH,
        0,
        UsageSummary::default(),
        archived,
        false,
        String::new(),
    )
    .with_app_session_id(app_session_id)
}

#[gpui::test]
fn prepared_active_rail_splices_archive_and_restore_in_display_order(
    cx: &mut gpui::TestAppContext,
) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::prepared_active_rail_splices_archive_and_restore_in_display_order"
        ),
        cx,
        |cx, app, runtime, project| {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let fixture = [
                        session("first", 1, project, false),
                        session("second", 2, project, false),
                        session("third", 3, project, false),
                    ];
                    let target = fixture[1].catalog_key();
                    let identity = fixture
                        .each_ref()
                        .map(super::super::RailSessionIdentity::for_session);
                    let neighbor = |rows: &super::super::PreparedActiveRows, index: usize| {
                        rows.replacement(&identity[index])
                            .map(|item| item.app_session_id())
                    };
                    app.sessions.all = fixture.to_vec().into();
                    app.sessions.visible = fixture.to_vec().into();
                    app.sessions.drafts.clear();
                    app.sessions.selected_draft = None;
                    app.sessions.order = vec![3, 1, 2];
                    for grouped in [false, true] {
                        app.settings.group_sessions_by_project = grouped;
                        app.notify_session_rail(cx);
                        let ids = |rows: &super::super::PreparedActiveRows| {
                            rows.visible_items()
                                .map(ActiveSessionItem::app_session_id)
                                .collect::<Vec<_>>()
                        };
                        let before = app.prepared_active_rows(cx);
                        assert_eq!(ids(&before), [3, 1, 2]);
                        assert_eq!(neighbor(&before, 2), Some(1));
                        assert_eq!(neighbor(&before, 0), Some(2));
                        assert_eq!(neighbor(&before, 1), Some(1));
                        LIST_BUILDS.set(0);
                        for status in [
                            crate::runtime::RunStatus::Working,
                            crate::runtime::RunStatus::Done,
                        ] {
                            runtime.send_event(crate::runtime::RuntimeEvent::SessionStatus {
                                target: crate::sessions::session_target(&fixture[0].path),
                                session: Some(fixture[0].path.clone()),
                                record: Some(crate::agent_activity::SessionStatusRecord::for_row(
                                    &fixture[0],
                                )),
                                status,
                            });
                            app.drain_runtime(cx);
                            assert_eq!(app.rail_row_status(&fixture[0], false), status);
                            assert_eq!(ids(&app.prepared_active_rows(cx)), [3, 1, 2]);
                        }
                        app.select_rail_session(target.clone(), window, cx);
                        assert_eq!(ids(&app.prepared_active_rows(cx)), [3, 1, 2]);
                        app.request_rail_archive(target.clone(), true, window, cx);
                        let archived = app.prepared_active_rows(cx);
                        assert_eq!((ids(&archived), archived.archived_count), (vec![3, 1], 1));
                        assert_eq!(neighbor(&archived, 0), Some(3));
                        assert_eq!(neighbor(&archived, 1), None);
                        app.request_rail_archive(target.clone(), false, window, cx);
                        let restored = app.prepared_active_rows(cx);
                        assert_eq!(
                            (ids(&restored), restored.archived_count),
                            (vec![3, 1, 2], 0)
                        );
                        assert_eq!(neighbor(&restored, 1), Some(1));
                        assert_eq!(
                            LIST_BUILDS.get(),
                            0,
                            "a warm switch, archive and restore should reuse the sorted rows"
                        );
                    }
                })
            });
        },
    );
}
