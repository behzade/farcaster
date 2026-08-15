//! GPUI projection of the bounded transcript surface.

use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement as _, ListSizingBehavior,
    ListState, ParentElement as _, StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
    list, prelude::FluentBuilder as _,
};

use crate::{
    app::PiApp,
    conversation::{TranscriptItem, TranscriptKind},
    primitives::{ButtonTone, button},
    theme::THEME,
};

const MAX_TOOL_CHARS: usize = 12_000;
const MAX_EXPANDED_TOOL_CHARS: usize = 200_000;

pub(crate) fn render(
    list_state: &ListState,
    following: bool,
    unseen: usize,
    entity: WeakEntity<PiApp>,
    cx: &mut Context<PiApp>,
) -> AnyElement {
    let jump = entity;
    let row_entity = jump.clone();
    let rows = list(
        list_state.clone(),
        cx.processor(move |this, index, _, _| {
            this.transcript_item(index)
                .map(|(item, expanded)| render_item(index, &item, expanded, row_entity.clone()))
                .unwrap_or_else(|| div().into_any_element())
        }),
    )
    .with_sizing_behavior(ListSizingBehavior::Auto)
    .w_full()
    .max_w(THEME.layout.transcript_max)
    .flex_grow_1();
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(
            div()
                .id("transcript-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_hidden()
                .flex()
                .justify_center()
                .bg(THEME.colors.canvas)
                .child(rows),
        )
        .when(!following, |root| {
            root.child(
                div()
                    .flex_none()
                    .flex()
                    .justify_center()
                    .bg(THEME.colors.canvas)
                    .py(THEME.space.xs)
                    .child(button(
                        "jump-to-latest",
                        if unseen == 0 {
                            "Jump to latest".to_owned()
                        } else {
                            format!("Jump to latest · {unseen} new")
                        },
                        ButtonTone::Accent,
                        true,
                        move |_, cx| {
                            let _ = jump.update(cx, |this, cx| this.jump_to_latest(cx));
                        },
                    )),
            )
        })
        .into_any_element()
}

fn render_item(
    index: usize,
    item: &TranscriptItem,
    expanded: bool,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let (color, compact) = match item.kind {
        TranscriptKind::User | TranscriptKind::Assistant => (THEME.colors.text, false),
        TranscriptKind::Thinking => (THEME.colors.subtle, true),
        TranscriptKind::Tool => (
            if item.is_error {
                THEME.colors.error
            } else {
                THEME.colors.muted
            },
            true,
        ),
        TranscriptKind::Error => (THEME.colors.error, false),
        TranscriptKind::Notice | TranscriptKind::Custom => (THEME.colors.muted, true),
    };
    let is_tool = item.kind == TranscriptKind::Tool;
    let is_thinking = item.kind == TranscriptKind::Thinking;
    let display_limit = if expanded {
        MAX_EXPANDED_TOOL_CHARS
    } else {
        MAX_TOOL_CHARS
    };
    let truncated = is_tool && item.text.chars().count() > display_limit;
    let mut text = if truncated {
        item.text.chars().take(display_limit).collect::<String>()
    } else {
        item.text.clone()
    };
    if truncated {
        text.push_str(if expanded {
            "\n… output truncated at the expanded-view safety limit"
        } else {
            "\n… output truncated; expand to read more"
        });
    }
    let can_expand = is_thinking || (is_tool && item.text.chars().count() > MAX_TOOL_CHARS);
    let toggle = entity;
    let label = if can_expand {
        format!("{} {}", item.label, if expanded { "‹" } else { "›" })
    } else {
        item.label.clone()
    };
    let gutter = if can_expand {
        div()
            .id(("toggle-transcript-item", index))
            .role(gpui::Role::Button)
            .aria_label(if expanded {
                "Collapse transcript item"
            } else {
                "Expand transcript item"
            })
            .tab_index(0)
            .w(THEME.layout.transcript_label_width)
            .flex_none()
            .text_right()
            .text_size(THEME.type_scale.caption)
            .font_weight(FontWeight::MEDIUM)
            .text_color(THEME.colors.subtle)
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                let _ = toggle.update(cx, |this, cx| this.toggle_transcript_item(index, cx));
            })
            .child(label)
            .into_any_element()
    } else {
        div()
            .w(THEME.layout.transcript_label_width)
            .flex_none()
            .text_right()
            .text_size(THEME.type_scale.caption)
            .font_weight(FontWeight::MEDIUM)
            .text_color(THEME.colors.subtle)
            .child(label)
            .into_any_element()
    };
    div()
        .id(("transcript-item", index))
        .w_full()
        .flex()
        .items_start()
        .gap(THEME.space.sm)
        .px(THEME.space.md)
        .py(if compact {
            THEME.space.xs
        } else {
            THEME.space.sm
        })
        .bg(THEME.colors.canvas)
        .border_b(THEME.border)
        .border_color(THEME.colors.border)
        .when(is_tool && (item.is_error || item.streaming), |row| {
            row.border_l(THEME.space.xs).border_color(if item.is_error {
                THEME.colors.error
            } else {
                THEME.colors.warning
            })
        })
        .child(gutter)
        .child(
            div()
                .id(("transcript-body", index))
                .min_w_0()
                .flex_1()
                .when(is_tool, |body| {
                    body.max_h(THEME.layout.tool_max_height).overflow_y_scroll()
                })
                .when(is_tool, |body| body.font_family("monospace"))
                .text_size(THEME.type_scale.body_small)
                .line_height(THEME.type_scale.line_body)
                .text_color(color)
                .child(if is_thinking && !expanded {
                    "Collapsed".to_owned()
                } else if text.is_empty() {
                    "…".to_owned()
                } else {
                    text
                }),
        )
        .into_any_element()
}
