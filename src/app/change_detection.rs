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

pub(in crate::app) fn inactive_session_catalog_changed(
    current: &[SessionSummary],
    current_all: &[SessionSummary],
    next: &[SessionSummary],
    next_all: &[SessionSummary],
) -> bool {
    let rows = |sessions: &[SessionSummary]| {
        crate::sessions::root_sessions(sessions)
            .into_iter()
            .filter(|session| session.archived)
            .map(|session| {
                (
                    session.id.clone(),
                    session.app_session_id,
                    session.path.clone(),
                    session.project.clone(),
                    session.title.clone(),
                    session.modified,
                    session.is_running,
                )
            })
            .collect::<Vec<_>>()
    };
    let current_rows = rows(current);
    if current_rows != rows(next) {
        return true;
    }
    let paths = current_rows
        .iter()
        .map(|(_, _, path, ..)| path.as_path())
        .collect::<HashSet<_>>();
    let waiting = |sessions| {
        roots_waiting_for_descendants(sessions)
            .into_iter()
            .filter(|path| paths.contains(path.as_path()))
            .collect::<HashSet<_>>()
    };
    let counts = |sessions: &[SessionSummary]| {
        let index = SessionRootIndex::new(sessions);
        let mut counts = HashMap::<PathBuf, usize>::new();
        for session in sessions {
            if let Some(parent) = index.parent(session)
                && paths.contains(parent.path.as_path())
            {
                *counts.entry(parent.path.clone()).or_default() += 1;
            }
        }
        counts
    };
    waiting(current_all) != waiting(next_all) || counts(current_all) != counts(next_all)
}

pub(in crate::app) fn session_event_affects_active_rail(
    drafts: &[sessions::DraftSession],
    submitted_drafts: &HashMap<String, Option<PathBuf>>,
    sessions: &[SessionSummary],
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
    let session = session_path
        .and_then(|path| sessions.iter().find(|session| session.path == path))
        .or_else(|| {
            sessions
                .iter()
                .find(|session| session_target(&session.path) == target)
        });
    session
        .and_then(|session| root_session_for_path(sessions, Some(&session.path)))
        .is_some_and(|root| !root.archived)
}

pub(in crate::app) fn run_panel_sessions_changed(
    current: &[SessionSummary],
    next: &[SessionSummary],
    selected: Option<&Path>,
) -> bool {
    visible_sessions_changed(current, next, selected, |left, right| {
        left.key() == right.key()
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
    })
}

pub(in crate::app) fn composer_usage_sessions_changed(
    current: &[SessionSummary],
    next: &[SessionSummary],
    selected: Option<&Path>,
) -> bool {
    visible_sessions_changed(current, next, selected, |left, right| {
        left.id == right.id
            && left.path == right.path
            && left.parent_session == right.parent_session
            && left.usage == right.usage
    })
}

pub(in crate::app) fn visible_sessions_changed(
    current: &[SessionSummary],
    next: &[SessionSummary],
    selected: Option<&Path>,
    equal: impl Fn(&SessionSummary, &SessionSummary) -> bool,
) -> bool {
    fn visible<'a>(
        sessions: &'a [SessionSummary],
        selected: Option<&Path>,
    ) -> Vec<(&'a SessionSummary, usize)> {
        let Some(root) = root_session_for_path(sessions, selected) else {
            return Vec::new();
        };
        let mut result = vec![(root, 0)];
        result.extend(descendant_sessions_for_root(sessions, root));
        result
    }

    let current = visible(current, selected);
    let next = visible(next, selected);
    current.len() != next.len()
        || current
            .iter()
            .zip(next)
            .any(|((left, left_depth), (right, right_depth))| {
                left_depth != &right_depth || !equal(left, right)
            })
}

pub(in crate::app) fn run_panel_activities_changed(
    current: &HashMap<String, crate::agent_activity::AgentActivity>,
    next: Option<&(HashMap<String, crate::agent_activity::AgentActivity>, bool)>,
    sessions: &[SessionSummary],
    selected: Option<&Path>,
) -> bool {
    let Some((activities, exhaustive)) = next else {
        return false;
    };
    let Some(root) = root_session_for_path(sessions, selected) else {
        return false;
    };
    let visible_keys = std::iter::once(root)
        .chain(
            descendant_sessions_for_root(sessions, root)
                .into_iter()
                .map(|(session, _)| session),
        )
        .map(|session| crate::agent_activity::agent_activity_key(&session.path))
        .collect::<Vec<_>>();
    visible_keys.into_iter().any(|key| {
        activities
            .get(&key)
            .is_some_and(|activity| current.get(&key) != Some(activity))
            || (*exhaustive && current.contains_key(&key) && !activities.contains_key(&key))
    })
}

pub(in crate::app) fn session_rail_snapshot_changed(
    roots: &SessionRootIndex<'_>,
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
    roots: &SessionRootIndex<'_>,
    previous: &RuntimeSnapshot,
    next: &RuntimeSnapshot,
) -> bool {
    let root_path = |path| {
        roots
            .root_for_path(path)
            .filter(|session| session.archived)
            .map(|session| session.path.as_path())
    };
    if root_path(previous.selected_session.as_deref())
        != root_path(next.selected_session.as_deref())
    {
        return true;
    }
    let previous_live = root_path(previous.live_session.as_deref());
    let next_live = root_path(next.live_session.as_deref());
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
        || previous.prompt_queue() != next.prompt_queue()
        || previous.conversation.average_cache_hit_rate != next.conversation.average_cache_hit_rate
        || previous.stats != next.stats
        || previous.account_usage != next.account_usage
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
    previous: &RuntimeSnapshot,
    next: &RuntimeSnapshot,
) -> bool {
    previous.selected_session != next.selected_session
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
