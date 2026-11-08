use gpui::{
    AnyElement, ClipboardItem, Context, FocusHandle, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, WeakEntity,
    Window, div, relative,
};
use gpui_component::text::{TextView, TextViewStyle};

use crate::app::{
    FarcasterApp, PostRenderFocus,
    ui::{
        assets::AppIcon,
        primitives::{ButtonTone, icon_button, modal},
        theme::{MONO_FONT_FAMILY, theme},
    },
};

#[derive(Clone, Copy)]
pub(in crate::app) enum CommandDetailKind {
    Script,
    Output,
}

pub(in crate::app) struct CommandDetail {
    title: &'static str,
    text: SharedString,
    markdown: SharedString,
    cwd: Option<SharedString>,
    pub(in crate::app) focus: FocusHandle,
    return_focus: Option<FocusHandle>,
}

impl FarcasterApp {
    pub(in crate::app) fn open_command_detail(
        &mut self,
        key: usize,
        kind: CommandDetailKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(item) = self.snapshot.conversation.items.get(key) else {
            return;
        };
        let Some(details) = &item.tool_details else {
            return;
        };
        let (title, text) = match kind {
            CommandDetailKind::Script => (
                "Script",
                details.display_command().unwrap_or_default().into_owned(),
            ),
            CommandDetailKind::Output => ("Output", item.tool_output.clone()),
        };
        let fence = "`".repeat(
            text.split(|ch| ch != '`')
                .map(str::len)
                .max()
                .unwrap_or(0)
                .max(2)
                + 1,
        );
        let preview = CommandDetail {
            title,
            markdown: format!("{fence}\n{text}\n{fence}").into(),
            text: text.into(),
            cwd: details
                .arguments
                .get("cwd")
                .and_then(serde_json::Value::as_str)
                .map(|cwd| cwd.to_owned().into()),
            focus: cx.focus_handle(),
            return_focus: window.focused(cx),
        };
        self.cover_native_workspace_surface(cx);
        self.overlays.command_detail = Some(preview);
        self.overlays.post_render_focus = Some(PostRenderFocus::CommandDetail);
        cx.notify();
    }

    pub(in crate::app) fn close_command_detail(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = self.overlays.command_detail.take() else {
            return;
        };
        self.restore_overlay_focus(detail.return_focus, &detail.focus, window, cx);
        self.restore_active_native_workspace_surface(window, cx);
        cx.notify();
    }
}

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
) -> Option<AnyElement> {
    let detail = app.overlays.command_detail.as_ref()?;
    Some(preview(detail, move |window, cx| {
        let _ = entity.update(cx, |app, cx| app.close_command_detail(window, cx));
    }))
}

fn preview(
    detail: &CommandDetail,
    dismiss: impl Fn(&mut Window, &mut gpui::App) + Clone + 'static,
) -> AnyElement {
    let close = dismiss.clone();
    let copy = detail.text.clone();
    let mut style = TextViewStyle::default();
    style.code_block = gpui::StyleRefinement::default()
        .p_0()
        .border_0()
        .bg(theme().colors.panel);
    style.code_block.text.font_size = Some(theme().type_scale.body_small.into());
    style.code_block.text.line_height = Some(theme().type_scale.line_code.into());
    modal(
        "command-detail",
        detail.title,
        &detail.focus,
        crate::app::OVERLAY_KEY_CONTEXT,
        dismiss,
        |surface| {
            surface
                .debug_selector(|| "command-detail-surface".into())
                .w(theme().size(760.0))
                .max_h(relative(0.8))
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(theme().space.sm)
                        .p(theme().space.sm)
                        .child(div().flex_1().child(detail.title))
                        .child(icon_button(
                            "copy-command-detail",
                            AppIcon::Copy,
                            "Copy full text",
                            ButtonTone::Quiet,
                            move |_, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.to_string()))
                            },
                        ))
                        .child(icon_button(
                            "close-command-detail",
                            AppIcon::X,
                            "Close command details",
                            ButtonTone::Quiet,
                            move |window, cx| {
                                close(window, cx);
                            },
                        )),
                )
                .children(detail.cwd.clone().map(|cwd| {
                    div()
                        .px(theme().space.md)
                        .text_size(theme().type_scale.caption)
                        .text_color(theme().colors.muted)
                        .child(cwd)
                }))
                .child(
                    div()
                        .id("command-detail-content")
                        .min_h_0()
                        .overflow_y_scroll()
                        .p(theme().space.md)
                        .child(
                            TextView::markdown("command-detail-text", detail.markdown.clone())
                                .style(style)
                                .selectable(true)
                                .font_family(MONO_FONT_FAMILY)
                                .text_size(theme().type_scale.body_small)
                                .text_color(theme().colors.text)
                                .w_full(),
                        ),
                )
        },
    )
    .into_any_element()
}

#[cfg(test)]
#[path = "command_detail_tests.rs"]
mod tests;
