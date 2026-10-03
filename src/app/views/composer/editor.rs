use gpui::{
    AnyElement, App, Entity, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    ParentElement as _, RenderOnce, Styled as _, WeakEntity, div, point, px,
};
use gpui_component::ElementExt as _;
use gpui_component::input::{
    Editor, EditorState, MoveDown, MoveUp, Paste, TextDecoration, TextDecorationCollection,
};

use super::super::FarcasterApp;
use crate::app::ui::theme::{UI_FONT_FAMILY, theme};
use crate::app::{
    COMPOSER_KEY_CONTEXT, ComposerCompletionNext, ComposerCompletionPrevious, ComposerHistoryNext,
    ComposerHistoryPrevious,
};

#[derive(IntoElement)]
pub(super) struct ComposerInput {
    composer: Entity<EditorState>,
    app: WeakEntity<FarcasterApp>,
    suggestion_count: usize,
    actions: AnyElement,
}

impl ComposerInput {
    pub(super) fn new(
        composer: Entity<EditorState>,
        app: WeakEntity<FarcasterApp>,
        suggestion_count: usize,
        actions: AnyElement,
    ) -> Self {
        Self {
            composer,
            app,
            suggestion_count,
            actions,
        }
    }
}

impl RenderOnce for ComposerInput {
    fn render(self, window: &mut gpui::Window, cx: &mut App) -> impl IntoElement {
        let highlights = self.app.upgrade().map(|app| {
            let app = app.read(cx);
            (
                app.composer.decorations.clone(),
                crate::app::composer::highlighting::decorations(
                    &self.composer.read(cx).value(),
                    &app.snapshot.commands,
                    &app.composer.project_files,
                ),
            )
        });
        let presentation = window.use_keyed_state(
            gpui::ElementId::NamedInteger(
                "composer-presentation".into(),
                self.composer.entity_id().as_u64(),
            ),
            cx,
            |_, _| ComposerPresentation::default(),
        );
        let value = self.composer.read(cx).value();
        if let Some((collection, decorations)) = highlights {
            presentation.update(cx, |state, cx| {
                state.update_highlights(&collection, &value, decorations, cx)
            });
        }
        let height = composer_height(&value, presentation.read(cx).width, window);
        let current_view = window.current_view();
        let measured = presentation.clone();
        let previous_history_entity = self.app.clone();
        let next_history_entity = self.app.clone();
        let previous_completion_entity = self.app.clone();
        let next_completion_entity = self.app.clone();
        let paste_entity = self.app.clone();
        let key_entity = self.app.clone();
        let cursor_entity = self.app.clone();
        let composer_for_paste = self.composer.clone();
        let suggestion_count = self.suggestion_count;
        let mut key_context = gpui::KeyContext::default();
        key_context.add(COMPOSER_KEY_CONTEXT);
        if suggestion_count > 0 {
            key_context.add("Completions");
        }

        div()
            .id("composer-input")
            .key_context(key_context)
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h(theme().size(48.0))
            .font_family(UI_FONT_FAMILY)
            .text_size(theme().type_scale.reading)
            .line_height(theme().type_scale.line_composer)
            .pl(theme().space.sm)
            .pr(theme().size(48.0))
            .capture_action(move |_: &Paste, _, cx| {
                if paste_entity
                    .update(cx, |this, cx| {
                        this.paste_composer_image(cx) || this.paste_composer_text(cx)
                    })
                    .unwrap_or(false)
                {
                    cx.stop_propagation();
                    return;
                }

                let composer = composer_for_paste.clone();
                cx.defer(move |cx| {
                    composer.update(cx, |input, cx| {
                        let offset = input.scroll_offset();
                        input.set_scroll_offset(point(offset.x, px(-1.0e9)), cx);
                    });
                });
            })
            .on_action(move |_: &ComposerHistoryPrevious, window, cx| {
                let handled = previous_history_entity
                    .update(cx, |this, cx| {
                        this.select_previous_composer_suggestion(suggestion_count, cx)
                            || this.handle_composer_history_key("up", window, cx)
                    })
                    .unwrap_or(false);
                if !handled {
                    window.dispatch_action(Box::new(MoveUp), cx);
                }
                cx.stop_propagation();
            })
            .on_action(move |_: &ComposerHistoryNext, window, cx| {
                let handled = next_history_entity
                    .update(cx, |this, cx| {
                        this.select_next_composer_suggestion(suggestion_count, cx)
                            || this.handle_composer_history_key("down", window, cx)
                    })
                    .unwrap_or(false);
                if !handled {
                    window.dispatch_action(Box::new(MoveDown), cx);
                }
                cx.stop_propagation();
            })
            .on_action(move |_: &ComposerCompletionPrevious, _, cx| {
                let _ = previous_completion_entity.update(cx, |this, cx| {
                    this.select_previous_composer_suggestion(suggestion_count, cx);
                });
                cx.stop_propagation();
            })
            .on_action(move |_: &ComposerCompletionNext, _, cx| {
                let _ = next_completion_entity.update(cx, |this, cx| {
                    this.select_next_composer_suggestion(suggestion_count, cx);
                });
                cx.stop_propagation();
            })
            .capture_key_down(move |_: &KeyDownEvent, _, cx| {
                capture_after_input(key_entity.clone(), cx);
            })
            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                capture_after_input(cursor_entity.clone(), cx);
            })
            .child(
                composer_editor(&self.composer, height).on_prepaint(move |bounds, _, cx| {
                    // Kit adds padding inside the editor state, independently
                    // of the outer frame's style. Shape at the text area's width.
                    let padding = gpui_component::Size::default().input_px();
                    let width = bounds.size.width - padding - padding.min(px(6.));
                    if measured.read(cx).width != Some(width) {
                        measured.update(cx, |state, _| state.width = Some(width));
                        cx.notify(current_view);
                    }
                }),
            )
            .child(self.actions)
    }
}

fn capture_after_input(entity: WeakEntity<FarcasterApp>, cx: &mut App) {
    cx.defer(move |cx| {
        let _ = entity.update(cx, |this, cx| this.capture_composer_session(cx));
    });
}

#[derive(Default)]
struct ComposerPresentation {
    width: Option<gpui::Pixels>,
    value: gpui::SharedString,
    decorations: Vec<TextDecoration>,
}

impl ComposerPresentation {
    fn update_highlights(
        &mut self,
        collection: &TextDecorationCollection,
        value: &gpui::SharedString,
        decorations: Vec<TextDecoration>,
        cx: &mut App,
    ) {
        let ranges: Vec<_> = decorations
            .iter()
            .map(|decoration| decoration.range.clone())
            .collect();
        let intact = collection.get_ranges(cx) == ranges;
        if self.value == *value && self.decorations == decorations && intact {
            return;
        }
        collection.set(decorations.clone(), cx);
        self.value = value.clone();
        self.decorations = decorations;
    }
}

fn composer_height(
    value: &gpui::SharedString,
    width: Option<gpui::Pixels>,
    window: &gpui::Window,
) -> gpui::Pixels {
    let font_size = theme().type_scale.reading;
    let line_height = theme().type_scale.line_composer;
    let run = gpui::TextRun {
        len: value.len(),
        font: gpui::font(UI_FONT_FAMILY),
        color: theme().colors.text.into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let rows = window
        .text_system()
        .shape_text(
            value.clone(),
            font_size,
            &[run],
            width.filter(|width| *width > px(0.0)),
            Some(8),
        )
        .map(|lines| {
            lines
                .iter()
                .map(|line| line.wrap_boundaries().len() + 1)
                .sum::<usize>()
        })
        .unwrap_or_else(|_| value.lines().count());
    line_height * rows.clamp(1, 8)
}

fn composer_editor(input: &Entity<EditorState>, height: gpui::Pixels) -> gpui::Stateful<gpui::Div> {
    let newline = input.clone();
    // Keep the requested number of text rows after Kit's internal padding.
    let height = height + gpui_component::Size::default().input_py() * 2.;
    div()
        .id("composer-editor")
        .w_full()
        .h(height)
        .capture_action(move |action: &gpui_component::input::Enter, window, cx| {
            if action.shift {
                use gpui::EntityInputHandler as _;
                newline.update(cx, |input, cx| {
                    input.replace_text_in_range(None, "\n", window, cx);
                    cx.emit(gpui_component::input::InputEvent::PressEnter {
                        secondary: action.secondary,
                        shift: true,
                    });
                });
                cx.stop_propagation();
            }
        })
        .child(
            Editor::new(input)
                .w_full()
                .h(height)
                .font_family(UI_FONT_FAMILY)
                .text_size(theme().type_scale.reading)
                .line_height(theme().type_scale.line_composer)
                .appearance(false)
                .p_0(),
        )
}

#[cfg(test)]
#[path = "editor_tests.rs"]
mod tests;
