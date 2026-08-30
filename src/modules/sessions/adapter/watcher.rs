use std::{fs, path::Path, sync::mpsc, thread};

use notify::{
    Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _, event::ModifyKind,
};

use super::super::SessionWatchEvent;

pub(crate) struct SessionWatcher {
    _watcher: RecommendedWatcher,
}

impl SessionWatcher {
    pub(crate) fn start(
        root: &Path,
        sender: mpsc::Sender<SessionWatchEvent>,
        wake: thread::Thread,
    ) -> Result<Self, String> {
        fs::create_dir_all(root)
            .map_err(|error| format!("prepare Pi session directory {}: {error}", root.display()))?;
        let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
            let event = match result {
                Ok(event) => session_event(&event),
                Err(error) => Some(SessionWatchEvent::Failed(format!(
                    "watch Pi sessions: {error}"
                ))),
            };
            if event.is_some_and(|event| sender.send(event).is_ok()) {
                wake.unpark();
            }
        })
        .map_err(|error| format!("create Pi session watcher: {error}"))?;
        watcher
            .watch(root, RecursiveMode::Recursive)
            .map_err(|error| format!("watch {}: {error}", root.display()))?;
        Ok(Self { _watcher: watcher })
    }
}

fn session_event(event: &Event) -> Option<SessionWatchEvent> {
    if matches!(event.kind, EventKind::Access(_)) {
        return None;
    }
    let paths = event
        .paths
        .iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
        })
        .cloned()
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return None;
    }
    match event.kind {
        EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Metadata(_)) => {
            Some(SessionWatchEvent::Activity(paths))
        }
        EventKind::Create(_)
        | EventKind::Remove(_)
        | EventKind::Modify(ModifyKind::Name(_))
        | EventKind::Modify(ModifyKind::Any | ModifyKind::Other)
        | EventKind::Any
        | EventKind::Other => Some(SessionWatchEvent::CatalogChanged),
        EventKind::Access(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use notify::{
        Event,
        event::{
            AccessKind, CreateKind, DataChange, MetadataKind, ModifyKind, RemoveKind, RenameMode,
        },
    };

    use super::*;

    #[test]
    fn watcher_prepares_a_missing_session_root() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().join("nested/sessions");
        let (sender, _receiver) = mpsc::channel();

        let _watcher = SessionWatcher::start(&root, sender, thread::current()).expect("watcher");

        assert!(root.is_dir());
    }

    #[test]
    fn classifies_catalog_activity_and_ignored_events() {
        let path = PathBuf::from("session.jsonl");
        for kind in [
            EventKind::Create(CreateKind::File),
            EventKind::Remove(RemoveKind::File),
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            EventKind::Any,
        ] {
            assert_eq!(
                session_event(&Event::new(kind).add_path(path.clone())),
                Some(SessionWatchEvent::CatalogChanged)
            );
        }
        for kind in [
            ModifyKind::Data(DataChange::Content),
            ModifyKind::Metadata(MetadataKind::Any),
        ] {
            assert_eq!(
                session_event(&Event::new(EventKind::Modify(kind)).add_path(path.clone())),
                Some(SessionWatchEvent::Activity(vec![path.clone()]))
            );
        }
        assert_eq!(
            session_event(&Event::new(EventKind::Access(AccessKind::Any)).add_path(path)),
            None
        );
        assert_eq!(
            session_event(&Event::new(EventKind::Any).add_path(PathBuf::from("settings.json"))),
            None
        );
    }
}
