use crate::app::infrastructure::persistence::ComposerAttachment;

pub(crate) use crate::sessions::{
    ComposerSnapshot, HistoryNavigation, draft_target, project_target, session_target,
};
pub(crate) type ComposerSessions = crate::sessions::ComposerSessions<ComposerAttachment>;

pub(in crate::app) fn catalog_row_for_buffer<'a>(
    buffer: &str,
    catalog: &'a crate::sessions::SessionCatalog,
) -> Option<&'a crate::sessions::SessionSummary> {
    let path = crate::sessions::session_path(crate::sessions::composer_command_target(buffer))?;
    let harness = crate::sessions::composer_buffer_parts(buffer).map(|(harness, _)| harness);
    let mut rows = catalog
        .at_path(path)
        .filter(|row| harness.is_none_or(|h| h == row.harness));
    let row = rows.next()?;
    rows.next().is_none().then_some(row)
}

pub(crate) fn load(
    current_target: String,
    cx: &mut gpui::Context<crate::app::FarcasterApp>,
) -> (ComposerSessions, Option<String>) {
    let (records, error) =
        match crate::app::persistence::open().and_then(|store| store.load_composer_buffers()) {
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
