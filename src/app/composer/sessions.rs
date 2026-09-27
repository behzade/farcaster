use crate::app::infrastructure::persistence::ComposerAttachment;

pub(crate) use crate::sessions::{
    ComposerSnapshot, HistoryNavigation, draft_target, project_target, session_target,
};
pub(crate) type ComposerSessions = crate::sessions::ComposerSessions<ComposerAttachment>;

pub(crate) fn load(
    current_target: String,
    cx: &mut gpui::Context<crate::app::FarcasterApp>,
) -> (ComposerSessions, Option<String>) {
    let (records, error) =
        match crate::app::persistence::open().and_then(|store| store.load_composer_sessions()) {
            Ok(records) => (records, None),
            Err(error) => (Vec::new(), Some(error)),
        };
    let (persistence, updates) =
        crate::storage::ComposerPersistenceWorker::spawn(crate::app::persistence::shared);
    crate::app::session::state_writer::observe_updates(
        crate::app::session::state_writer::PersistenceSource::Composer,
        updates,
        cx,
    );
    (
        ComposerSessions::new(current_target, records, Box::new(persistence)),
        error,
    )
}
