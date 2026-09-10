use super::*;
use crate::app::ui::theme::{THEME, TRANSCRIPT_FONT_SIZE_RANGE};

impl StateStore {
    pub(crate) fn load_transcript_font_size(&self) -> Result<f32, String> {
        self.connection
            .query_row(
                "SELECT value FROM meta WHERE key='transcript_font_size'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map(|value| {
                value
                    .and_then(|value| value.parse::<f32>().ok())
                    .filter(|size| TRANSCRIPT_FONT_SIZE_RANGE.contains(size))
                    .unwrap_or(f32::from(THEME.type_scale.reading))
            })
            .map_err(|error| format!("load transcript font size: {error}"))
    }

    pub(crate) fn save_transcript_font_size(&self, size: f32) -> Result<(), String> {
        if !TRANSCRIPT_FONT_SIZE_RANGE.contains(&size) {
            return Err("Transcript font size must be between 10 and 32 px.".into());
        }
        self.connection
            .execute(
                "INSERT INTO meta(key, value) VALUES('transcript_font_size', ?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                [size.to_string()],
            )
            .map(|_| ())
            .map_err(|error| format!("save transcript font size: {error}"))
    }
}

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod tests;
