use crate::runtime::RunStatus;

use super::*;

pub(in crate::app) fn transcript_follow_state_needs_update(
    current: bool,
    unseen: usize,
    following: bool,
) -> bool {
    current != following || (following && unseen != 0)
}

pub(in crate::app) fn update_transcript_follow_state(
    current: &mut bool,
    unseen: &mut usize,
    following: bool,
) -> bool {
    let changed = transcript_follow_state_needs_update(*current, *unseen, following);
    *current = following;
    if following {
        *unseen = 0;
    }
    changed
}

pub(in crate::app) fn session_catalog_changed(
    current: &[SessionSummary],
    current_all: &[SessionSummary],
    current_error: Option<&str>,
    next: &[SessionSummary],
    next_all: &[SessionSummary],
) -> bool {
    current != next || current_all != next_all || current_error.is_some()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::app) enum ArchiveChange {
    Unchanged,
    Repaint,
    Rebuild,
}

fn archive_display_changed(left: &SessionSummary, right: &SessionSummary) -> bool {
    left.title != right.title
        || left.harness != right.harness
        || left.path != right.path
        || left.id != right.id
        || left.profile_id != right.profile_id
        || left.is_running != right.is_running
        || left.model != right.model
        || left.thinking_level != right.thinking_level
        || left.usage != right.usage
        || left.first_user_message != right.first_user_message
}

pub(in crate::app) fn archive_catalog_change(
    current: &sessions::SessionCatalog,
    current_all: &sessions::SessionCatalog,
    next: &sessions::SessionCatalog,
    next_all: &sessions::SessionCatalog,
) -> ArchiveChange {
    fn archived(catalog: &sessions::SessionCatalog) -> impl Iterator<Item = &SessionSummary> {
        catalog.roots().filter(|row| row.archived)
    }
    let mut before = archived(current);
    let mut after = archived(next);
    let mut repaint = false;
    let mut has_rows = false;
    loop {
        match (before.next(), after.next()) {
            (Some(left), Some(right)) => {
                if left.catalog_key() != right.catalog_key()
                    || left.project != right.project
                    || left.modified != right.modified
                {
                    return ArchiveChange::Rebuild;
                }
                has_rows = true;
                if !repaint {
                    repaint = archive_display_changed(left, right)
                        || current_all.child_count(left) != next_all.child_count(right)
                        || current_all.has_shared_path(&left.path)
                            != next_all.has_shared_path(&right.path);
                }
            }
            (None, None) => break,
            _ => return ArchiveChange::Rebuild,
        }
    }
    if has_rows && !repaint {
        let before = roots_waiting_for_descendants(current_all);
        let after = roots_waiting_for_descendants(next_all);
        repaint = before != after
            && archived(next).any(|row| {
                let key = row.catalog_key();
                before.contains(&key) != after.contains(&key)
            });
    }
    if repaint {
        ArchiveChange::Repaint
    } else {
        ArchiveChange::Unchanged
    }
}

// A new or reparented record may resolve or orphan other rows. A metadata update
// with stable links only affects its own archive row or its family's badge.
pub(in crate::app) fn archive_row_change(
    catalog: &sessions::SessionCatalog,
    next: &SessionSummary,
) -> ArchiveChange {
    let Some(old) = catalog.matching_record(next) else {
        return ArchiveChange::Rebuild;
    };
    if old.app_session_id != next.app_session_id
        || old.path != next.path
        || old.harness != next.harness
        || old.id != next.id
        || old.project != next.project
        || old.profile_id != next.profile_id
        || old.parent_app_session_id != next.parent_app_session_id
        || old.parent_session != next.parent_session
        || old.parent_harness != next.parent_harness
    {
        return ArchiveChange::Rebuild;
    }
    let root = catalog.root_for_row(old);
    if root.is_some_and(|root| root.same_record(old)) {
        if old.archived != next.archived || (next.archived && old.modified != next.modified) {
            return ArchiveChange::Rebuild;
        }
        // Unbound rows can tie on both archive sort fields. Upsert moves the
        // edited row to the front of its recency group, changing stable tie order.
        if next.archived && next.app_session_id <= 0 {
            let first = catalog.partition_point(|row| row.modified > next.modified);
            if catalog.get(first).is_some_and(|row| !row.same_record(old)) {
                return ArchiveChange::Rebuild;
            }
        }
    }
    if root.is_some_and(|root| root.archived)
        && (old.is_running != next.is_running
            || (old.archived && archive_display_changed(old, next)))
    {
        ArchiveChange::Repaint
    } else {
        ArchiveChange::Unchanged
    }
}

pub(in crate::app) fn session_event_affects_active_rail(
    drafts: &[sessions::DraftSession],
    submitted_drafts: &HashMap<String, Option<PathBuf>>,
    sessions: &crate::sessions::SessionCatalog,
    target: &str,
    session_path: Option<&Path>,
) -> bool {
    if target
        .strip_prefix("draft:")
        .is_some_and(|id| drafts.iter().any(|draft| draft.id == id))
        || submitted_drafts
            .values()
            .flatten()
            .any(|path| session_path == Some(path.as_path()) || session_target(path) == target)
    {
        return true;
    }
    session_path
        .and_then(|path| sessions.root_for_path(Some(path)))
        .or_else(|| sessions.root_for_path(crate::sessions::session_path(target)))
        .is_some_and(|root| !root.archived)
}

#[cfg(test)]
pub(in crate::app) fn run_panel_sessions_changed(
    current: &sessions::SessionCatalog,
    next: &sessions::SessionCatalog,
    selected: Option<&Path>,
) -> bool {
    run_panel_family_changed(
        current,
        next,
        current.root_for_path(selected),
        next.root_for_path(selected),
    )
}

pub(in crate::app) fn run_panel_family_changed(
    current: &sessions::SessionCatalog,
    next: &sessions::SessionCatalog,
    current_root: Option<&SessionSummary>,
    next_root: Option<&SessionSummary>,
) -> bool {
    visible_sessions_changed(
        current,
        next,
        current_root,
        next_root,
        |left, right, depth| {
            // Worker cards retain the full summary for their tooltip and fallback activity.
            if depth > 0 {
                return left == right;
            }
            left.app_session_id == right.app_session_id
                && left.worker_profile == right.worker_profile
                && left.archived == right.archived
                && left.harness == right.harness
                && left.profile_id == right.profile_id
                && left.id == right.id
                && left.path == right.path
                && left.project == right.project
                && left.timestamp == right.timestamp
                && left.created_at == right.created_at
                && left.parent_session == right.parent_session
                && left.is_running == right.is_running
                && left.model == right.model
                && left.thinking_level == right.thinking_level
        },
    )
}

pub(in crate::app) fn composer_usage_sessions_changed(
    current: &sessions::SessionCatalog,
    next: &sessions::SessionCatalog,
    selected: Option<&Path>,
) -> bool {
    visible_sessions_changed(
        current,
        next,
        current.root_for_path(selected),
        next.root_for_path(selected),
        |left, right, _| {
            left.id == right.id
                && left.path == right.path
                && left.parent_session == right.parent_session
                && left.usage == right.usage
        },
    )
}

pub(in crate::app) fn visible_sessions_changed(
    current: &sessions::SessionCatalog,
    next: &sessions::SessionCatalog,
    current_root: Option<&SessionSummary>,
    next_root: Option<&SessionSummary>,
    equal: impl Fn(&SessionSummary, &SessionSummary, usize) -> bool,
) -> bool {
    fn visible<'a>(
        sessions: &'a sessions::SessionCatalog,
        root: Option<&'a SessionSummary>,
    ) -> Vec<(&'a SessionSummary, usize)> {
        let Some(root) = root else {
            return Vec::new();
        };
        let mut result = vec![(root, 0)];
        result.extend(sessions.descendants(root));
        result
    }

    let current = visible(current, current_root);
    let next = visible(next, next_root);
    current.len() != next.len()
        || current
            .iter()
            .zip(next)
            .any(|((left, left_depth), (right, right_depth))| {
                left_depth != &right_depth || !equal(left, right, right_depth)
            })
}

pub(in crate::app) fn session_rail_snapshot_changed(
    roots: &sessions::SessionCatalog,
    previous: &RuntimeSnapshot,
    next: &RuntimeSnapshot,
) -> bool {
    let root_path = |path| {
        roots
            .root_for_path(path)
            .map(|session| session.path.as_path())
    };
    root_path(previous.selected_session.as_deref()) != root_path(next.selected_session.as_deref())
        || root_path(previous.live_session.as_deref()) != root_path(next.live_session.as_deref())
        || previous.live_status != next.live_status
}

pub(in crate::app) fn inactive_session_rail_snapshot_changed(
    roots: &sessions::SessionCatalog,
    previous: &RuntimeSnapshot,
    next: &RuntimeSnapshot,
) -> bool {
    let archived_root = |snapshot: &RuntimeSnapshot, path: Option<&Path>| {
        let row = snapshot.catalog_row_at(roots, path?)?;
        roots
            .root_for_row(row)
            .filter(|root| root.archived)
            .map(SessionSummary::catalog_key)
    };
    if archived_root(previous, previous.selected_session.as_deref())
        != archived_root(next, next.selected_session.as_deref())
    {
        return true;
    }
    let previous_live = archived_root(previous, previous.live_session.as_deref());
    let next_live = archived_root(next, next.live_session.as_deref());
    previous_live != next_live || (previous.live_status != next.live_status && next_live.is_some())
}

pub(in crate::app) fn composer_snapshot_changed(
    previous: &RuntimeSnapshot,
    next: &RuntimeSnapshot,
) -> bool {
    previous.conversation.items.is_empty() != next.conversation.items.is_empty()
        || previous.selected_session != next.selected_session
        || previous.commands != next.commands
        || previous.conversation.running != next.conversation.running
        || previous.conversation.compacting != next.conversation.compacting
        || previous.prompt_queue() != next.prompt_queue()
        || previous.conversation.average_cache_hit_rate != next.conversation.average_cache_hit_rate
        || previous.stats != next.stats
        || previous.pending_question != next.pending_question
        || previous.session_identity() != next.session_identity()
        || previous.selected_service_tier() != next.selected_service_tier()
        || previous.available_service_tiers() != next.available_service_tiers()
        || previous.models != next.models
        || previous.thinking_levels != next.thinking_levels
        || previous.configuration_status != next.configuration_status
        || previous.access_mode != next.access_mode
        || (!Arc::ptr_eq(&previous.conversation, &next.conversation)
            && previous.conversation.pending_receipts_ref()
                != next.conversation.pending_receipts_ref())
}

pub(in crate::app) fn run_panel_snapshot_changed(
    catalog: &sessions::SessionCatalog,
    previous: &RuntimeSnapshot,
    next: &RuntimeSnapshot,
) -> bool {
    previous.selected_session != next.selected_session
        || next.selected_session.as_deref().is_some_and(|path| {
            let selected = |snapshot: &RuntimeSnapshot| {
                snapshot
                    .catalog_row_at(catalog, path)
                    .map(SessionSummary::catalog_key)
            };
            selected(previous) != selected(next)
        })
}

pub(in crate::app) fn input_snapshot(input: &EditorState) -> ComposerSnapshot {
    ComposerSnapshot::new(
        input.value().to_string(),
        input.cursor(),
        input.selected_range(),
    )
}

pub(in crate::app) fn park_extension_surface(
    visible: &mut ExtensionUiState,
    parked: &mut Option<ExtensionUiState>,
) {
    if parked.is_none() {
        *parked = Some(std::mem::take(visible));
    }
}

pub(in crate::app) fn restore_extension_surface(
    visible: &mut ExtensionUiState,
    parked: &mut Option<ExtensionUiState>,
) {
    if let Some(session) = parked.take() {
        *visible = session;
    }
}

pub(in crate::app) fn starts_recent_completion(
    previous: Option<&RunStatus>,
    next: &RunStatus,
    force: bool,
) -> bool {
    *next == RunStatus::Done && (force || previous.is_some_and(|status| *status != RunStatus::Done))
}

pub(in crate::app) fn session_identities_changed(
    current: &[SessionSummary],
    next: &[SessionSummary],
) -> bool {
    let identities = |sessions: &[SessionSummary]| {
        sessions
            .iter()
            .map(|session| (session.path.clone(), session.key()))
            .collect::<HashMap<_, _>>()
    };
    identities(current) != identities(next)
}
