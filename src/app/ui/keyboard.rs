use gpui::{ClipboardItem, Context, Keystroke, Window, actions};

use crate::app::{AppSurface, FarcasterApp};

actions!(
    farcaster,
    [CopySelection, ClipboardCopyAlias, ClipboardPasteAlias]
);

impl FarcasterApp {
    pub(in crate::app) fn copy_selection(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = copy_text(
            self.transcript_selected_text(window, cx),
            self.composer.read(cx).selected_value().to_string(),
        ) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub(in crate::app) fn handle_clipboard_alias(
        &mut self,
        paste: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let native = matches!(self.surface, AppSurface::Editor | AppSurface::Terminal);
        if !paste && !native {
            self.copy_selection(window, cx);
            return;
        }

        let target = match (native, paste) {
            (true, false) => "ctrl-shift-c",
            (true, true) => "ctrl-shift-v",
            (false, true) => "ctrl-v",
            (false, false) => return,
        };
        if let Ok(keystroke) = Keystroke::parse(target) {
            window.defer(cx, move |window, cx| {
                window.dispatch_keystroke(keystroke, cx);
            });
        }
    }
}

fn copy_text(transcript: Option<String>, composer: String) -> Option<String> {
    transcript
        .filter(|text| !text.is_empty())
        .or_else(|| (!composer.is_empty()).then_some(composer))
}

#[cfg(test)]
#[path = "keyboard_tests.rs"]
mod tests;
