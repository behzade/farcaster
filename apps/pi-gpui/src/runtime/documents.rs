//! Authoritative in-memory session document lifecycle.

use std::time::SystemTime;

use super::*;

type SessionDocumentRevision = (SystemTime, usize);

pub(super) fn session_document_is_live(
    session: &SessionSummary,
    interacted: bool,
    rpc_attached: bool,
) -> bool {
    rpc_attached || session.is_running || (interacted && !session.settled)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reconcile_live_session_documents(
    sessions: &[SessionSummary],
    interacted: &HashSet<String>,
    selected: &str,
    actors: &mut HashMap<String, SessionRuntimeHandle>,
    documents: &mut HashMap<String, Arc<RuntimeSnapshot>>,
    last_touch: &mut HashMap<String, u64>,
    revisions: &mut HashMap<PathBuf, SessionDocumentRevision>,
    actor_paths: &mut HashMap<PathBuf, String>,
    process_command: &ProcessCommand,
    supervisor: &thread::Thread,
) {
    let mut session_keys = HashSet::new();
    let mut live_keys = HashSet::new();
    for session in sessions {
        let requested_key = format!("session:{}", session.path.display());
        let key = actor_paths
            .get(&session.path)
            .cloned()
            .unwrap_or(requested_key);
        session_keys.insert(key.clone());
        let rpc_attached = documents.get(&key).is_some_and(|snapshot| {
            snapshot.connected && !snapshot.history_preview && snapshot.live_session.is_some()
        });
        if !session_document_is_live(session, interacted.contains(&key), rpc_attached) {
            continue;
        }
        live_keys.insert(key.clone());
        let revision = (session.modified, session.message_count);
        if revisions.get(&session.path) == Some(&revision) || rpc_attached {
            continue;
        }
        revisions.insert(session.path.clone(), revision);
        let actor = actors.entry(key.clone()).or_insert_with(|| {
            SessionRuntimeHandle::spawn(
                session.project.clone(),
                process_command.clone(),
                false,
                supervisor.clone(),
            )
        });
        actor_paths.insert(session.path.clone(), key);
        actor.send(RuntimeCommand::RefreshSessionDocument {
            path: session.path.clone(),
            project: session.project.clone(),
        });
    }

    let stale = actors
        .keys()
        .filter(|key| {
            key.as_str() != selected && session_keys.contains(*key) && !live_keys.contains(*key)
        })
        .cloned()
        .collect::<Vec<_>>();
    for key in stale {
        if documents.get(&key).is_some_and(|snapshot| {
            snapshot.connected && !snapshot.history_preview && snapshot.live_session.is_some()
        }) {
            continue;
        }
        if let Some(actor) = actors.remove(&key) {
            actor.send(RuntimeCommand::Shutdown);
        }
        if let Some(snapshot) = documents.remove(&key)
            && let Some(path) = snapshot.selected_session.as_ref()
        {
            revisions.remove(path);
        }
        last_touch.remove(&key);
        actor_paths.retain(|_, actor| actor != &key);
    }
}
