use crate::app::infrastructure::persistence::ComposerAttachment;

pub(crate) use crate::sessions::{
    ComposerSnapshot, HistoryNavigation, draft_target, project_target, session_target,
};
pub(crate) type ComposerSessions = crate::sessions::ComposerSessions<ComposerAttachment>;

pub(crate) fn load(current_target: String) -> (ComposerSessions, Option<String>) {
    let (records, error) =
        match crate::app::persistence::open().and_then(|store| store.load_composer_sessions()) {
            Ok(records) => (records, None),
            Err(error) => (Vec::new(), Some(error)),
        };
    let persistence =
        crate::storage::ComposerPersistenceWorker::spawn(crate::app::persistence::shared());
    (
        ComposerSessions::new(current_target, records, Box::new(persistence)),
        error,
    )
}
