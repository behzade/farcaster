use gpui::{App, ClipboardItem, Context, Keystroke, Window, actions};

use crate::app::{AppSurface, FarcasterApp};

actions!(
    farcaster,
    [CopySelection, ClipboardCopyAlias, ClipboardPasteAlias]
);

pub(crate) fn copy_selection(transcript: Option<String>, composer: String, cx: &mut App) {
    if let Some(text) = copy_text(transcript, composer) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
}

impl FarcasterApp {
    pub(in crate::app) fn handle_clipboard_alias(
        &mut self,
        paste: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let native = matches!(self.surface, AppSurface::Editor | AppSurface::Terminal);
        if !paste && !native {
            copy_selection(
                self.transcript_list.selected_text(),
                self.composer.read(cx).selected_value().to_string(),
                cx,
            );
            return;
        }

        let target = match (native, paste) {
            (true, false) => "ctrl-shift-c",
            (true, true) => "ctrl-shift-v",
            (false, true) => "ctrl-v",
            (false, false) => return,
        };
        if let Ok(keystroke) = Keystroke::parse(target) {
            cx.defer_in(window, move |_, window, cx| {
                window.dispatch_keystroke(keystroke, cx);
            });
        }
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
