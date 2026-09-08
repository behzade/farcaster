mod catalog;
mod metrics;
mod path;
mod persistence;

pub(crate) use catalog::{
    SessionRootIndex, archived_root_family_for_path, descendant_sessions, document_is_live,
    filter_session_tree, is_subagent_path, root_session_for_path, root_sessions,
    session_family_for_path,
};
pub(crate) use metrics::{
    CatalogMetrics, count_cache_hit, count_parse, count_scan, take_catalog_metrics,
};
pub(crate) use path::{normalize_lexical, normalize_session_path};
pub(crate) use persistence::{
    SessionStore, cached as cached_sessions, delete as delete_state, index as index_sessions,
    relocate as relocate_state, set_archived,
};
