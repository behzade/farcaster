//! Chrome for the embedded Neovim editor surface.

use gpui::{AnyElement, App, IntoElement as _, ParentElement as _, Styled as _, WeakEntity, div};

use super::super::PiApp;
use crate::{
    primitives::{ButtonTone, button},
    theme::{MONO_FONT_FAMILY, THEME},
};

impl PiApp {
    pub(super) fn render_editor_surface(&self, entity: WeakEntity<Self>, cx: &App) -> AnyElement {
        let path = self.editor_path(cx);
        let title = path
            .as_deref()
            .and_then(|path| path.strip_prefix(&self.project).ok())
            .unwrap_or_else(|| {
                path.as_deref()
                    .unwrap_or_else(|| std::path::Path::new("Editor"))
            })
            .display()
            .to_string();
        let content = if let Some(editor) = self.editor.clone() {
            editor.into_any_element()
        } else {
            div()
                .size_full()
                .p(THEME.space.md)
                .text_color(THEME.colors.error)
                .child(
                    self.editor_error
                        .clone()
                        .unwrap_or_else(|| "Neovim is not running".to_owned()),
                )
                .into_any_element()
        };
        let back = entity.clone();
        let close = entity;
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(gpui::px(44.0))
                    .flex_none()
                    .px(THEME.space.sm)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(THEME.space.sm)
                    .border_b(THEME.border)
                    .border_color(THEME.colors.border)
                    .bg(THEME.colors.panel)
                    .child(button(
                        "editor-back-to-chat",
                        "Back to chat",
                        ButtonTone::Quiet,
                        true,
                        move |window, cx| {
                            let _ = back.update(cx, |app, cx| app.show_chat_surface(window, cx));
                        },
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .font_family(MONO_FONT_FAMILY)
                            .text_size(THEME.type_scale.caption)
                            .text_color(THEME.colors.muted)
                            .child(title),
                    )
                    .child(button(
                        "editor-close",
                        "Close editor",
                        ButtonTone::Quiet,
                        true,
                        move |window, cx| {
                            let _ = close.update(cx, |app, cx| app.close_editor(window, cx));
                        },
                    )),
            )
            .child(div().flex_1().min_h_0().child(content))
            .into_any_element()
    }
}
