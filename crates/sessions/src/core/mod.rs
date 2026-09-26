mod catalog;
mod catalog_cache;
pub use catalog_cache::SessionCatalog;
mod metrics;
mod path;
mod persistence;

#[cfg(test)]
pub use catalog::descendant_sessions;
pub use catalog::{
    SessionRootIndex, archived_root_family_for_path, descendant_sessions_for_root,
    document_is_live, filter_session_tree, is_subagent_path, root_session_for_path, root_sessions,
    session_family_for_path,
};
pub use metrics::{CatalogMetrics, count_cache_hit, count_parse, count_scan, take_catalog_metrics};
pub use path::{normalize_lexical, normalize_session_path};
pub use persistence::{
    DraftStore, SessionStore, cached as cached_sessions, delete as delete_state,
    index as index_sessions, load_drafts, relocate as relocate_state, remove_draft, save_draft,
    set_archived,
};
