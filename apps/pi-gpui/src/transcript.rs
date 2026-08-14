//! GPUI projection of the bounded transcript surface.

use std::collections::HashSet;

use gpui::{
    AnyElement, FontWeight, InteractiveElement as _, IntoElement as _, ParentElement as _,
    ScrollDelta, ScrollHandle, StatefulInteractiveElement as _, Styled as _, WeakEntity, div,
    prelude::FluentBuilder as _,
};

use crate::{
    app::PiApp,
    conversation::{TranscriptItem, TranscriptKind},
    primitives::{ButtonTone, button},
    theme::THEME,
};

pub(crate) const DEFAULT_VISIBLE_ITEMS: usize = 300;
const VISIBLE_PAGE_ITEMS: usize = 300;
const MAX_VISIBLE_ITEMS: usize = 1_200;
const MAX_TOOL_CHARS: usize = 12_000;
const MAX_EXPANDED_TOOL_CHARS: usize = 200_000;

pub(crate) fn render(
    items: &[TranscriptItem],
    scroll: &ScrollHandle,
    visible_items: usize,
    expanded: &HashSet<usize>,
    following: bool,
    unseen: usize,
    entity: WeakEntity<PiApp>,
) -> AnyElement {
    let visible_items = visible_items.min(MAX_VISIBLE_ITEMS);
    let omitted = items.len().saturating_sub(visible_items);
    let rows = items
        .iter()
        .enumerate()
        .skip(omitted)
        .map(|(index, item)| render_item(index, item, expanded.contains(&index), entity.clone()))
        .collect::<Vec<_>>();
    let pause = entity.clone();
    let reveal = entity.clone();
    let jump = entity;
    let total_items = items.len();
    let can_reveal_more = visible_items < MAX_VISIBLE_ITEMS;
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(
            div()
                .id("transcript-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(scroll)
                .on_scroll_wheel(move |event, _, cx| {
                    if scrolls_toward_older(event.delta) {
                        let _ = pause.update(cx, |this, cx| this.pause_transcript_follow(cx));
                    }
                })
                .bg(THEME.colors.canvas)
                .child(
                    div()
                        .w_full()
                        .max_w(THEME.layout.transcript_max)
                        .mx_auto()
                        .py(THEME.space.md)
                        .when(omitted > 0, |content| {
                            content.child(div().px(THEME.space.md).py(THEME.space.sm).child(
                                button(
                                    "reveal-older-transcript",
                                    if can_reveal_more {
                                        format!(
                                            "Show {} more older transcript items",
                                            omitted.min(VISIBLE_PAGE_ITEMS)
                                        )
                                    } else {
                                        format!("{omitted} older transcript items remain hidden")
                                    },
                                    ButtonTone::Quiet,
                                    can_reveal_more,
                                    move |_, cx| {
                                        let _ = reveal.update(cx, |this, cx| {
                                            this.reveal_older_transcript(total_items, cx)
                                        });
                                    },
                                ),
                            ))
                        })
                        .children(rows),
                ),
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
    let (surface, color, compact) = match item.kind {
        TranscriptKind::User => (THEME.colors.panel, THEME.colors.text, false),
        TranscriptKind::Assistant => (THEME.colors.canvas, THEME.colors.text, false),
        TranscriptKind::Thinking => (THEME.colors.canvas, THEME.colors.subtle, true),
        TranscriptKind::Tool => (
            THEME.colors.panel,
            if item.is_error {
                THEME.colors.error
            } else {
                THEME.colors.muted
            },
            true,
        ),
        TranscriptKind::Error => (THEME.colors.panel, THEME.colors.error, false),
        TranscriptKind::Notice | TranscriptKind::Custom => {
            (THEME.colors.panel, THEME.colors.muted, true)
        }
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
    div()
        .id(("transcript-item", index))
        .w_full()
        .px(THEME.space.md)
        .py(if compact {
            THEME.space.xs
        } else {
            THEME.space.sm
        })
        .bg(surface)
        .when(is_tool, |row| {
            row.border_l(THEME.space.xs).border_color(if item.is_error {
                THEME.colors.error
            } else if item.streaming {
                THEME.colors.warning
            } else {
                THEME.colors.accent
            })
        })
        .child(
            div()
                .mb(THEME.space.xs)
                .flex()
                .items_center()
                .justify_between()
                .gap(THEME.space.xs)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .text_size(THEME.type_scale.caption)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(color)
                        .child(item.label.clone())
                        .when(item.streaming, |label| label.child(" · running")),
                )
                .when(can_expand, |header| {
                    header.child(button(
                        ("toggle-transcript-item", index),
                        if expanded { "Collapse" } else { "Expand" },
                        ButtonTone::Quiet,
                        true,
                        move |_, cx| {
                            let _ = toggle
                                .update(cx, |this, cx| this.toggle_transcript_item(index, cx));
                        },
                    ))
                }),
        )
        .child(
            div()
                .id(("transcript-body", index))
                .when(is_tool, |body| {
                    body.max_h(THEME.layout.tool_max_height).overflow_y_scroll()
                })
                .when(is_tool, |body| body.font_family("monospace"))
                .text_size(THEME.type_scale.body)
                .line_height(THEME.type_scale.line_body)
                .text_color(color)
                .child(if is_thinking && !expanded {
                    "Thinking is collapsed".to_owned()
                } else if text.is_empty() {
                    "…".to_owned()
                } else {
                    text
                }),
        )
        .into_any_element()
}

pub(crate) fn next_visible_limit(current: usize, total: usize) -> usize {
    current
        .saturating_add(VISIBLE_PAGE_ITEMS)
        .min(MAX_VISIBLE_ITEMS)
        .min(total.max(DEFAULT_VISIBLE_ITEMS))
}

pub(crate) fn scrolls_toward_older(delta: ScrollDelta) -> bool {
    match delta {
        ScrollDelta::Pixels(point) => point.y > gpui::px(0.),
        ScrollDelta::Lines(point) => point.y > 0.,
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_VISIBLE_ITEMS, next_visible_limit, scrolls_toward_older};
    use gpui::{ScrollDelta, point, px};

    #[test]
    fn only_upward_history_scroll_pauses_following() {
        assert!(scrolls_toward_older(ScrollDelta::Pixels(point(
            px(0.),
            px(1.)
        ))));
        assert!(!scrolls_toward_older(ScrollDelta::Lines(point(0., -1.))));
    }

    #[test]
    fn older_transcript_reveals_in_pages_with_a_hard_render_cap() {
        assert_eq!(next_visible_limit(300, 10_000), 600);
        assert_eq!(next_visible_limit(600, 750), 750);
        assert_eq!(
            next_visible_limit(MAX_VISIBLE_ITEMS, 10_000),
            MAX_VISIBLE_ITEMS
        );
    }
}
