use gpui::{IntoElement, ParentElement as _, Styled as _, WeakEntity, div};

use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};

use crate::app::{
    FarcasterApp,
    composer::sessions::ComposerSnapshot,
    ui::{
        primitives::{ButtonTone, button, dropdown_button},
        theme::THEME,
    },
};

pub(super) fn harness_selector(
    harness: &str,
    entity: WeakEntity<FarcasterApp>,
) -> impl IntoElement {
    let backends = crate::agents::backend_statuses()
        .into_iter()
        .filter(|backend| backend.available)
        .collect::<Vec<_>>();
    let selected = harness.to_owned();
    let label = backends
        .iter()
        .find(|backend| backend.id == harness)
        .map(|backend| backend.name.to_owned())
        .unwrap_or_else(|| harness.to_owned());
    dropdown_button("draft-harness", label, ButtonTone::Quiet, true)
        .text_color(THEME.colors.text)
        .dropdown_menu_with_anchor(gpui::Anchor::BottomLeft, move |mut menu, _, _| {
            for backend in &backends {
                let target = backend.id.clone();
                let entity = entity.clone();
                menu = menu.item(
                    PopupMenuItem::new(backend.name.clone())
                        .checked(backend.id == selected)
                        .on_click(move |_, window, cx| {
                            let _ = entity.update(cx, |this, cx| {
                                this.change_draft_harness(target.clone(), window, cx);
                            });
                        }),
                );
            }
            menu
        })
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
        .rounded_b(THEME.radius)
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
