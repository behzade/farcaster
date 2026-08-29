//! Central routing for application keyboard commands.

use gpui::{App, ClipboardItem, actions};

actions!(farcaster, [CopySelection]);

pub(crate) fn copy_selection(transcript: Option<String>, composer: String, cx: &mut App) {
    if let Some(text) = copy_text(transcript, composer) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
}

fn copy_text(transcript: Option<String>, composer: String) -> Option<String> {
    transcript
        .filter(|text| !text.trim().is_empty())
        .or_else(|| (!composer.is_empty()).then_some(composer))
}

#[cfg(test)]
mod tests {
    use super::copy_text;

    #[test]
    fn transcript_visual_selection_takes_copy_precedence() {
        assert_eq!(
            copy_text(Some("transcript".to_owned()), "composer".to_owned()),
            Some("transcript".to_owned())
        );
    }

    #[test]
    fn composer_selection_is_the_input_mode_fallback() {
        assert_eq!(
            copy_text(None, "composer".to_owned()),
            Some("composer".to_owned())
        );
        assert_eq!(copy_text(None, String::new()), None);
    }
}
