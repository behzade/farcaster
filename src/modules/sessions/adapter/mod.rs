mod deletion;
mod pi_files;
mod transfer;
mod watcher;

pub(crate) use pi_files::{
    configured_session_root, normalize_session_path, project_display_history,
};

pub(crate) fn delete_family(
    paths: &[std::path::PathBuf],
) -> Result<Vec<(std::path::PathBuf, String)>, String> {
    if paths
        .first()
        .is_some_and(|path| crate::agents::is_external_session(path))
    {
        for path in paths.iter().rev() {
            crate::agents::delete_external_session(path)
                .ok_or_else(|| "session family mixes backend locators".to_owned())??;
        }
        return Ok(Vec::new());
    }
    deletion::delete_family(paths)
}

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
