use super::StateStore;
use crate::app::ui::theme::{THEME, TRANSCRIPT_FONT_SIZE_RANGE};

impl StateStore {
    pub(crate) fn load_transcript_font_size(&self) -> Result<f32, String> {
        self.load_transcript_font_size_setting(
            &TRANSCRIPT_FONT_SIZE_RANGE,
            f32::from(THEME.type_scale.reading),
        )
    }

    pub(crate) fn save_transcript_font_size(&self, size: f32) -> Result<(), String> {
        self.save_transcript_font_size_setting(size, &TRANSCRIPT_FONT_SIZE_RANGE)
    }
}

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod tests;
