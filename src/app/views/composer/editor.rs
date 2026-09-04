use gpui::{
    AnyElement, App, Entity, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    ParentElement as _, RenderOnce, Styled as _, WeakEntity, div, point, px,
};
use gpui_component::input::{MoveDown, MoveUp, Paste, Textarea, TextareaState};

use super::super::FarcasterApp;
use crate::app::ui::theme::{MONO_FONT_FAMILY, THEME};
use crate::app::{
    COMPOSER_KEY_CONTEXT, ComposerCompletionNext, ComposerCompletionPrevious, ComposerHistoryNext,
    ComposerHistoryPrevious,
};

#[derive(IntoElement)]
pub(super) struct ComposerInput {
    composer: Entity<TextareaState>,
    app: WeakEntity<FarcasterApp>,
    suggestion_count: usize,
    actions: AnyElement,
}

impl ComposerInput {
    pub(super) fn new(
        composer: Entity<TextareaState>,
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
    fn render(self, _: &mut gpui::Window, _: &mut App) -> impl IntoElement {
        let previous_history_entity = self.app.clone();
        let next_history_entity = self.app.clone();
        let previous_completion_entity = self.app.clone();
        let next_completion_entity = self.app.clone();
        let paste_entity = self.app.clone();
        let key_entity = self.app.clone();
        let cursor_entity = self.app;
        let composer_for_paste = self.composer.clone();
        let suggestion_count = self.suggestion_count;

        div()
            .id("composer-input")
            .key_context(COMPOSER_KEY_CONTEXT)
            .relative()
            .flex_1()
            .min_h(px(112.0))
            .font_family(MONO_FONT_FAMILY)
            .text_size(THEME.type_scale.body)
            .line_height(THEME.type_scale.line_composer)
            .pl(THEME.space.sm)
            .pr(px(48.0))
            .py(THEME.space.sm)
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
            .child(Textarea::new(&self.composer).w_full().appearance(false))
            .child(self.actions)
    }
}

fn capture_after_input(entity: WeakEntity<FarcasterApp>, cx: &mut App) {
    cx.defer(move |cx| {
        let _ = entity.update(cx, |this, cx| this.capture_composer_session(cx));
    });
}
