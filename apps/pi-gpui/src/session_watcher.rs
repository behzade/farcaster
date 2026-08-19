//! Filesystem event source for Pi session catalog changes.

use std::{fs, path::Path, sync::mpsc, thread};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum SessionWatchEvent {
    Changed,
    Failed(String),
}

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
                Ok(event) if is_session_change(&event) => SessionWatchEvent::Changed,
                Ok(_) => return,
                Err(error) => SessionWatchEvent::Failed(format!("watch Pi sessions: {error}")),
            };
            if sender.send(event).is_ok() {
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

fn is_session_change(event: &Event) -> bool {
    !matches!(event.kind, EventKind::Access(_))
        && event.paths.iter().any(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
        })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use notify::{Event, event::AccessKind};

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
    fn only_non_access_jsonl_events_refresh_the_catalog() {
        let changed = Event::new(EventKind::Any).add_path(PathBuf::from("session.jsonl"));
        let unrelated = Event::new(EventKind::Any).add_path(PathBuf::from("settings.json"));
        let access =
            Event::new(EventKind::Access(AccessKind::Any)).add_path(PathBuf::from("session.jsonl"));

        assert!(is_session_change(&changed));
        assert!(!is_session_change(&unrelated));
        assert!(!is_session_change(&access));
    }
}
