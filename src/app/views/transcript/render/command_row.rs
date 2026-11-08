use super::*;
use crate::app::{
    ui::primitives::{ButtonTone, button, icon_button},
    views::dialogs::command_detail::CommandDetailKind,
};

pub(super) fn render(
    font_scale: f32,
    key: usize,
    command: &str,
    item: &TranscriptItem,
    expanded: bool,
    entity: WeakEntity<FarcasterApp>,
) -> AnyElement {
    let status = item_status(item);
    let label = command_label(command);
    let output = output_preview(&item.tool_output);
    let script = entity.clone();
    let result = entity.clone();
    let copy = entity.clone();
    div()
        .id(("command-row", key))
        .w_full()
        .px(theme().size(18.0))
        .py(theme().space.xs)
        .flex()
        .flex_col()
        .gap(theme().space.xs)
        .child(
            tool_changes::title_row(
                ("command-title", key),
                format!(
                    "{} {label} details. {}",
                    if expanded { "Collapse" } else { "Expand" },
                    status.map_or("No result", ToolStatus::label)
                ),
                toggle_transcript_item(entity, key, expanded),
            )
            .aria_expanded(expanded)
            .py(theme().space.xs)
            .child(status_slot(status))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_family(MONO_FONT_FAMILY)
                    .text_size(theme().type_scale.body_small * font_scale)
                    .line_height(theme().type_scale.line_body * font_scale)
                    .truncate()
                    .child(label.lines().next().unwrap_or_default().to_owned()),
            ),
        )
        .when(expanded, |row| {
            row.child(
                div()
                    .ml(theme().icons.control + theme().space.xs)
                    .flex()
                    .flex_col()
                    .gap(theme().space.xs)
                    .when(!output.is_empty(), |body| {
                        body.child(
                            div()
                                .font_family(MONO_FONT_FAMILY)
                                .text_size(theme().type_scale.body_small * font_scale)
                                .line_height(theme().type_scale.line_body * font_scale)
                                .line_clamp(3)
                                .text_ellipsis()
                                .text_color(theme().colors.text)
                                .child(output),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(theme().space.xs)
                            .child(
                                button(
                                    ("command-script", key),
                                    "Script",
                                    ButtonTone::Quiet,
                                    true,
                                    move |window, cx| {
                                        let _ = script.update(cx, |app, cx| {
                                            app.open_command_detail(
                                                key,
                                                CommandDetailKind::Script,
                                                window,
                                                cx,
                                            )
                                        });
                                    },
                                )
                                .child(app_icon(AppIcon::ArrowsOut, AppIconSize::Inline)),
                            )
                            .child(
                                button(
                                    ("command-output", key),
                                    "Output",
                                    ButtonTone::Quiet,
                                    !item.tool_output.is_empty(),
                                    move |window, cx| {
                                        let _ = result.update(cx, |app, cx| {
                                            app.open_command_detail(
                                                key,
                                                CommandDetailKind::Output,
                                                window,
                                                cx,
                                            )
                                        });
                                    },
                                )
                                .child(app_icon(AppIcon::ArrowsOut, AppIconSize::Inline)),
                            )
                            .child(div().flex_1())
                            .child(icon_button(
                                ("copy-command-script", key),
                                AppIcon::Copy,
                                "Copy script",
                                ButtonTone::Quiet,
                                move |_, cx| {
                                    let _ = copy.update(cx, |app, cx| {
                                        if let Some(script) =
                                            app.snapshot.conversation.items.get(key).and_then(
                                                |item| {
                                                    item.tool_details.as_ref()?.display_command()
                                                },
                                            )
                                        {
                                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                                script.into_owned(),
                                            ));
                                        }
                                    });
                                },
                            )),
                    ),
            )
        })
        .into_any_element()
}

fn output_preview(output: &str) -> String {
    let mut lines = output.rsplit('\n').filter(|line| !line.trim().is_empty());
    let mut tail = lines
        .by_ref()
        .take(3)
        .map(|line| {
            let mut chars = line.chars();
            let mut text: String = chars.by_ref().take(240).collect();
            if chars.next().is_some() {
                text.push('…');
            }
            text
        })
        .collect::<Vec<_>>();
    let omitted = lines.next().is_some();
    tail.reverse();
    let text = tail.join("\n");
    if omitted { format!("… {text}") } else { text }
}

#[cfg(test)]
#[path = "command_row_tests.rs"]
mod tests;
