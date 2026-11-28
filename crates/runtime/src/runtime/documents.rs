use std::{path::Path, time::SystemTime};

use super::*;

type SessionDocumentRevision = (PathBuf, SystemTime, usize);

#[derive(Default)]
pub(super) struct ActorPaths(HashMap<PathBuf, Vec<(Backend, String)>>);

impl ActorPaths {
    pub(super) fn get(&self, path: &Path, harness: Backend) -> Option<&String> {
        self.0
            .get(path)?
            .iter()
            .find_map(|(backend, key)| (*backend == harness).then_some(key))
    }

    pub(super) fn last_at(&self, path: &Path) -> Option<&String> {
        self.0.get(path)?.last().map(|(_, key)| key)
    }

    pub(super) fn resident(
        &self,
        path: &Path,
        harness: Backend,
        latest: &HashMap<String, Arc<RuntimeSnapshot>>,
    ) -> Option<&String> {
        self.get(path, harness).filter(|key| {
            latest
                .get(*key)
                .and_then(|snapshot| snapshot.harness)
                .is_none_or(|owner| owner == harness)
        })
    }

    pub(super) fn insert(&mut self, path: PathBuf, harness: Backend, key: String) {
        let owners = self.0.entry(path).or_default();
        owners.retain(|(backend, actor)| *backend != harness && *actor != key);
        owners.push((harness, key));
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (&PathBuf, &String)> {
        self.0
            .iter()
            .flat_map(|(path, owners)| owners.iter().map(move |(_, actor)| (path, actor)))
    }

    pub(super) fn retain(&mut self, keep: impl Fn(&PathBuf, &String) -> bool) {
        self.0.retain(|path, owners| {
            owners.retain(|(_, key)| keep(path, key));
            !owners.is_empty()
        });
    }

    pub(super) fn session_key(
        &self,
        path: &Path,
        harness: Backend,
        latest: &HashMap<String, Arc<RuntimeSnapshot>>,
    ) -> String {
        if let Some(key) = self.resident(path, harness, latest) {
            return key.clone();
        }
        self.available_key(path, harness, &sessions::session_target(path), latest)
    }

    pub(super) fn available_key(
        &self,
        path: &Path,
        harness: Backend,
        requested: &str,
        latest: &HashMap<String, Arc<RuntimeSnapshot>>,
    ) -> String {
        if self.0.get(path).is_some_and(|owners| {
            owners
                .iter()
                .any(|(owner, key)| *owner != harness && key == requested)
        }) || latest
            .get(requested)
            .and_then(|snapshot| snapshot.harness)
            .is_some_and(|owner| owner != harness)
        {
            return format!("session-record:{harness}:{path:?}");
        }
        requested.to_owned()
    }
}

pub(super) fn catalog_actor_key(
    row: &SessionSummary,
    catalog: &sessions::SessionCatalog,
    paths: &ActorPaths,
    latest: &HashMap<String, Arc<RuntimeSnapshot>>,
) -> String {
    if !catalog.has_shared_path(&row.path) {
        return paths.session_key(&row.path, row.harness, latest);
    }
    let matching = |snapshot: &RuntimeSnapshot| {
        (snapshot.selected_session.as_ref() == Some(&row.path)
            || snapshot.live_session.as_ref() == Some(&row.path))
            && snapshot.matches_session_row(row)
    };
    if let Some((key, _)) = latest.iter().find(|(_, snapshot)| matching(snapshot)) {
        return key.clone();
    }
    if let Some(key) = paths.resident(&row.path, row.harness, latest)
        && latest.get(key).is_none_or(|snapshot| {
            snapshot.selected_session.is_none() && snapshot.live_session.is_none()
        })
        && catalog
            .at_path(&row.path)
            .filter(|other| other.harness == row.harness)
            .count()
            == 1
    {
        return key.clone();
    }
    record_actor_key(&row.catalog_key())
}

pub(super) fn record_actor_key(key: &sessions::SessionKey) -> String {
    match key {
        sessions::SessionKey::App(id) => id.to_key(),
        locator => format!("session-record:{locator:?}"),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reconcile_live_session_documents(
    sessions: &[SessionSummary],
    catalog: &sessions::SessionCatalog,
    interacted: &HashSet<String>,
    selected: &str,
    actors: &mut HashMap<String, SessionRuntimeHandle>,
    documents: &mut HashMap<String, Arc<RuntimeSnapshot>>,
    last_touch: &mut HashMap<String, u64>,
    revisions: &mut HashMap<String, SessionDocumentRevision>,
    actor_paths: &mut ActorPaths,
    process_command: &AgentLaunchConfig,
    supervisor: &thread::Thread,
    host: &Arc<dyn RuntimeHost>,
) {
    let mut stale = HashSet::new();
    let mut live_keys = HashSet::new();
    for session in sessions {
        let key = catalog_actor_key(session, catalog, actor_paths, documents);
        let rpc_attached = documents.get(&key).is_some_and(|snapshot| {
            snapshot.connected && !snapshot.history_preview && snapshot.live_session.is_some()
        });
        if !sessions::document_is_live(session, interacted.contains(&key), rpc_attached) {
            if key != selected && actors.contains_key(&key) {
                stale.insert(key);
            }
            continue;
        }
        live_keys.insert(key.clone());
        let revision = (
            session.path.clone(),
            session.modified,
            session.message_count,
        );
        if rpc_attached {
            revisions.insert(key.clone(), revision);
            continue;
        }
        if revisions.get(&key) == Some(&revision) {
            continue;
        }
        revisions.insert(key.clone(), revision);
        let actor = actors.entry(key.clone()).or_insert_with(|| {
            SessionRuntimeHandle::spawn(
                session.project.clone(),
                process_command.clone(),
                false,
                Some(session.harness),
                supervisor.clone(),
                host.clone(),
            )
        });
        actor_paths.insert(session.path.clone(), session.harness, key);
        actor.send(RuntimeCommand::RefreshSessionDocument {
            path: session.path.clone(),
            project: session.project.clone(),
            harness: Some(session.harness),
        });
    }

    for key in stale {
        if live_keys.contains(&key) {
            continue;
        }
        if documents.get(&key).is_some_and(|snapshot| {
            snapshot.connected && !snapshot.history_preview && snapshot.live_session.is_some()
        }) {
            continue;
        }
        if let Some(actor) = actors.remove(&key) {
            actor.send(RuntimeCommand::Shutdown);
        }
        documents.remove(&key);
        revisions.remove(&key);
        last_touch.remove(&key);
        actor_paths.retain(|_, actor| actor != &key);
    }
}
