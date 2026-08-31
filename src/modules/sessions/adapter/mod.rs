mod deletion;
mod pi_files;
mod transfer;
mod watcher;

pub(crate) use deletion::delete_family;
pub(crate) use pi_files::{
    configured_session_root, normalize_session_path, project_display_history,
};

pub(crate) fn discover(query: &str) -> Result<super::SessionDiscovery, String> {
    let mut discovery = pi_files::discover(query)?;
    let (external, external_exhaustive) = crate::agents::discover_external_sessions(query);
    discovery.sessions.extend(external);
    discovery.exhaustive &= external_exhaustive;
    discovery
        .sessions
        .sort_by_key(|session| std::cmp::Reverse(session.modified));
    Ok(discovery)
}

pub(crate) fn load_history(path: &std::path::Path) -> Result<super::LoadedHistory, String> {
    crate::agents::load_external_history(path).unwrap_or_else(|| pi_files::load_history(path))
}
pub(crate) use transfer::{destination_directory, move_family};
pub(crate) use watcher::SessionWatcher;
