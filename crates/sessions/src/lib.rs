pub mod activity;
mod composer;
mod contract;
mod core;
mod draft;
mod folders;

pub use composer::{
    ComposerPersistence, ComposerRecord, ComposerSessions, ComposerSnapshot, HistoryNavigation,
    draft_id, draft_target, project_target, session_path, session_target,
};
pub use contract::{
    LoadedHistory, PromptDeliveryReconciliation, RUNNING_ACTIVITY_TIMEOUT, RestoredQuestion,
    SessionDiscovery, SessionImport, SessionSummary, SessionTarget, SessionTransfer,
    TransferMember, UsageSummary,
};
#[cfg(test)]
pub use core::descendant_sessions;
pub use core::{
    CatalogMetrics, DraftStore, SessionRootIndex, SessionStore, archived_root_family_for_path,
    cached_sessions, count_cache_hit, count_parse, count_scan, delete_state,
    descendant_sessions_for_root, document_is_live, filter_session_tree, index_sessions,
    is_subagent_path, load_drafts, normalize_lexical, normalize_session_path, relocate_state,
    remove_draft, root_session_for_path, root_sessions, save_draft, session_family_for_path,
    set_archived, take_catalog_metrics,
};
pub use draft::{
    DraftSession, establish_submission, fill_session_association, reconciliation_candidates,
    submitted_draft_associations, sync_materialized_draft, update_persisted_submission,
};
#[cfg(any(test, feature = "test-support"))]
pub use folders::SessionFolder;
pub use folders::{FolderDestination, SessionFolders};
