use gpui::{IntoElement, ParentElement as _, Styled as _, WeakEntity, div};

use crate::app::{
    FarcasterApp, PickerScope, ProjectPickerIntent,
    composer::sessions::ComposerSnapshot,
    ui::{
        primitives::{ButtonTone, button},
        theme::THEME,
    },
};

pub(super) fn harness_selector(
    harness: &str,
    entity: WeakEntity<FarcasterApp>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(THEME.space.xs)
        .children(
            crate::agents::backend_statuses()
                .into_iter()
                .filter(|backend| backend.available)
                .map(|backend| {
                    let selected = backend.id == harness;
                    let target = backend.id;
                    let entity = entity.clone();
                    button(
                        format!("draft-harness-{target}"),
                        backend.name,
                        if selected {
                            ButtonTone::Accent
                        } else {
                            ButtonTone::Quiet
                        },
                        !selected,
                        move |window, cx| {
                            let _ = entity.update(cx, |this, cx| {
                                this.change_draft_harness(target.clone(), window, cx);
                            });
                        },
                    )
                }),
        )
}

pub(super) fn shortcuts(entity: WeakEntity<FarcasterApp>) -> impl IntoElement {
    let files = entity.clone();
    let prompts = entity.clone();
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(THEME.space.sm)
        .px(THEME.space.sm)
        .py(THEME.space.xs)
        .bg(THEME.colors.canvas)
        .child(button(
            "draft-files",
            "Add files",
            ButtonTone::Quiet,
            true,
            move |window, cx| {
                let _ = files.update(cx, |this, cx| this.open_composer_browser('@', window, cx));
            },
        ))
        .child(button(
            "draft-prompts",
            "Browse prompts",
            ButtonTone::Quiet,
            true,
            move |window, cx| {
                let _ = prompts.update(cx, |this, cx| this.open_composer_browser('$', window, cx));
            },
        ))
        .child(button(
            "draft-project",
            "Choose project",
            ButtonTone::Quiet,
            true,
            move |window, cx| {
                let _ = entity.update(cx, |this, cx| {
                    this.open_picker(
                        PickerScope::Projects(ProjectPickerIntent::ChangeDraft),
                        window,
                        cx,
                    );
                });
            },
        ))
}

impl FarcasterApp {
    fn open_composer_browser(
        &mut self,
        sigil: char,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let input = self.composer.read(cx);
        let text = input.value().to_string();
        let cursor = input.cursor();
        let snapshot = browser_snapshot(&text, cursor, sigil);
        self.apply_composer_snapshot(snapshot, window, cx);
        self.composer_view
            .update(cx, |view, _| view.reset_suggestion_selection());
        self.capture_composer_session(cx);
        self.composer_focus.focus(window, cx);
        self.notify_composer(cx);
    }
}

fn browser_snapshot(text: &str, cursor: usize, sigil: char) -> ComposerSnapshot {
    let mut cursor = cursor.min(text.len());
    while !text.is_char_boundary(cursor) {
        cursor -= 1;
    }
    let prefix = &text[..cursor];
    let gap = if prefix.is_empty() || prefix.chars().next_back().is_some_and(char::is_whitespace) {
        ""
    } else {
        " "
    };
    let suffix = &text[cursor..];
    let trailing_gap = if suffix.is_empty() || suffix.starts_with(char::is_whitespace) {
        ""
    } else {
        " "
    };
    let value = format!("{prefix}{gap}{sigil}{trailing_gap}{suffix}");
    let cursor = prefix.len() + gap.len() + sigil.len_utf8();
    ComposerSnapshot::new(value, cursor, cursor..cursor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_browser_keeps_existing_text_and_places_cursor_on_query() {
        let snapshot = browser_snapshot("read this", 5, '@');
        assert_eq!(snapshot.text, "read @ this");
        assert_eq!(snapshot.cursor, 6);
        assert_eq!(browser_snapshot("", 0, '$').text, "$");
        assert_eq!(browser_snapshot("سلام", "سلام".len(), '$').text, "سلام $");
    }
}
