pub(crate) mod activity;
mod adapter;
mod contract;
mod core;

pub(crate) use adapter::{
    SessionWatcher, configured_session_root, delete_family, destination_directory, discover,
    load_history, move_family, normalize_session_path, project_display_history,
};
pub(crate) use contract::{
    ChangeSet, FileChange, FileChangeKind, LoadedHistory, RUNNING_ACTIVITY_TIMEOUT,
    SessionDiscovery, SessionSummary, SessionTarget, SessionTransfer, SessionWatchEvent,
    TransferMember, UsageSummary,
};
pub(crate) use core::{
    SessionRootIndex, SessionStore, archived_root_family_for_path, cached_sessions,
    collect_changes, delete_state, descendant_sessions, filter_session_tree, index_sessions,
    is_subagent_path, normalize_lexical, relocate_state, root_session_for_path, root_sessions,
    session_family_for_path, set_archived,
};
