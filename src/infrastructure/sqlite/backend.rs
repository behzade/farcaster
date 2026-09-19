use crate::agents::Backend;

pub(super) fn get(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Backend> {
    let value = row.get::<_, String>(index)?;
    value.parse().map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
        )
    })
}
