//! One custom GPUI element for the whole transcript.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Bounds, ContentMask, Context, Element, ElementId, Font, FontStyle, FontWeight,
    GlobalElementId, Hsla, InteractiveElement as _, IntoElement as _, LayoutId, ListSizingBehavior,
    ListState, MouseButton, ParentElement as _, Pixels, Point, SharedString, Style, Styled as _,
    TextAlign, TextRun, WeakEntity, Window, WrappedLine, div, fill, list, point, px, relative,
    size,
};

use crate::{
    app::PiApp,
    conversation::{TranscriptItem, TranscriptKind},
    primitives::{ButtonTone, button},
    theme::THEME,
};

const TOOL_VISUAL_LINES: usize = 3;
const TOOL_PREVIEW_CHARS: usize = 320;
const VISIBLE_OVERDRAW: f32 = 72.0;

pub(crate) fn render(
    list_state: &ListState,
    following: bool,
    unseen: usize,
    entity: WeakEntity<PiApp>,
    cx: &mut Context<PiApp>,
) -> AnyElement {
    let jump = entity.clone();
    let click = entity;
    let view = cx.entity();
    let rows = list(list_state.clone(), move |_, _, _| {
        TranscriptElement { view: view.clone() }.into_any_element()
    })
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
                .flex_1()
                .min_h_0()
                .overflow_y_hidden()
                .flex()
                .justify_center()
                .bg(THEME.colors.canvas)
                .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                    let _ =
                        click.update(cx, |this, cx| this.toggle_transcript_at(event.position, cx));
                })
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

struct TranscriptElement {
    view: gpui::Entity<PiApp>,
}

impl gpui::IntoElement for TranscriptElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TranscriptElement {
    type RequestLayoutState = ();
    type PrepaintState = TranscriptPaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let total_height = self.view.update(cx, |view, _| {
            let width = view.transcript_width;
            update_layout_cache(view, width, window);
            view.transcript_layout.total_height
        });
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = px(total_height.max(1.0)).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let width_changed = self.view.update(cx, |view, _| {
            view.transcript_bounds = Some(bounds);
            let changed =
                (f32::from(view.transcript_width) - f32::from(bounds.size.width)).abs() >= 0.5;
            if changed {
                view.transcript_width = bounds.size.width;
                view.transcript_layout.mark_dirty(0);
            }
            changed
        });
        if width_changed {
            window.refresh();
        }

        let mask = window.content_mask().bounds;
        self.view
            .update(cx, |view, _| build_paint_state(view, bounds, mask, window))
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(
            Some(ContentMask {
                bounds: prepaint.clip,
            }),
            |window| {
                for separator in prepaint.separators.drain(..) {
                    window.paint_quad(separator);
                }
                for item in prepaint.items.drain(..) {
                    if let Some(label) = item.label {
                        let _ = label.line.paint(
                            label.origin,
                            THEME.type_scale.line_body,
                            TextAlign::Right,
                            Some(label.bounds),
                            window,
                            cx,
                        );
                    }
                    let mut top = item.origin.y;
                    for line in item.lines {
                        let _ = line.paint(
                            point(item.origin.x, top),
                            THEME.type_scale.line_body,
                            TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                        top +=
                            px(visual_rows(&line) as f32 * f32::from(THEME.type_scale.line_body));
                    }
                }
            },
        );
    }
}

#[derive(Default)]
pub(crate) struct TranscriptLayoutCache {
    wrap_width: f32,
    dirty_from: Option<usize>,
    items: Vec<ItemLayout>,
    total_height: f32,
}

impl TranscriptLayoutCache {
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn mark_dirty(&mut self, index: usize) {
        self.dirty_from = Some(self.dirty_from.map_or(index, |dirty| dirty.min(index)));
    }

    pub(crate) fn thinking_item_at(
        &self,
        bounds: Bounds<Pixels>,
        position: Point<Pixels>,
    ) -> Option<usize> {
        if !bounds.contains(&position) {
            return None;
        }
        let y = f32::from(position.y - bounds.top());
        self.items
            .iter()
            .find(|item| item.thinking && y >= item.top && y < item.top + item.height)
            .map(|item| item.index)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ItemLayout {
    index: usize,
    top: f32,
    content_top: f32,
    height: f32,
    thinking: bool,
    separator: bool,
}

#[derive(Default)]
struct TranscriptPaintState {
    clip: Bounds<Pixels>,
    separators: Vec<gpui::PaintQuad>,
    items: Vec<ItemPaint>,
}

struct ItemPaint {
    origin: Point<Pixels>,
    lines: Vec<WrappedLine>,
    label: Option<LabelPaint>,
}

struct LabelPaint {
    line: WrappedLine,
    origin: Point<Pixels>,
    bounds: Bounds<Pixels>,
}

fn update_layout_cache(view: &mut PiApp, width: Pixels, window: &mut Window) {
    let width_f = f32::from(width);
    if (view.transcript_layout.wrap_width - width_f).abs() >= 0.5 {
        view.transcript_layout.mark_dirty(0);
        view.transcript_layout.wrap_width = width_f;
    }
    let Some(dirty_from) = view.transcript_layout.dirty_from.take() else {
        return;
    };
    let items = &view.snapshot.conversation.items;
    let dirty_from = dirty_from.min(items.len());
    view.transcript_layout.items.truncate(dirty_from);
    let mut top = view
        .transcript_layout
        .items
        .last()
        .map_or(0.0, |item| item.top + item.height);
    for (index, item) in items.iter().enumerate().skip(dirty_from) {
        let separator = starts_transcript_chunk(items, index);
        let compact = !matches!(item.kind, TranscriptKind::User | TranscriptKind::Assistant);
        let leading = if separator {
            f32::from(THEME.space.sm + THEME.space.md)
        } else {
            0.0
        };
        let padding = f32::from(if compact {
            THEME.space.xs
        } else {
            THEME.space.sm
        });
        let content_top = top + leading + padding;
        let expanded = view.expanded_transcript_items.contains(&index);
        let lines = shape_item(item, expanded, width, window);
        let content_height = lines
            .iter()
            .map(|line| visual_rows(line) as f32 * f32::from(THEME.type_scale.line_body))
            .sum::<f32>()
            .max(f32::from(THEME.type_scale.line_body));
        let height = leading + padding + content_height + padding;
        view.transcript_layout.items.push(ItemLayout {
            index,
            top,
            content_top,
            height,
            thinking: item.kind == TranscriptKind::Thinking,
            separator,
        });
        top += height;
    }
    view.transcript_layout.total_height = top;
}

fn build_paint_state(
    view: &mut PiApp,
    bounds: Bounds<Pixels>,
    mask: Bounds<Pixels>,
    window: &mut Window,
) -> TranscriptPaintState {
    let visible_top = (f32::from(mask.top() - bounds.top()) - VISIBLE_OVERDRAW).max(0.0);
    let visible_bottom = f32::from(mask.bottom() - bounds.top()) + VISIBLE_OVERDRAW;
    let start = view
        .transcript_layout
        .items
        .partition_point(|item| item.top + item.height <= visible_top);
    let end = view.transcript_layout.items[start..]
        .partition_point(|item| item.top < visible_bottom)
        + start;
    let mut state = TranscriptPaintState {
        clip: mask,
        ..TranscriptPaintState::default()
    };
    for layout in &view.transcript_layout.items[start..end] {
        let Some(item) = view.snapshot.conversation.items.get(layout.index) else {
            continue;
        };
        if layout.separator {
            state.separators.push(fill(
                Bounds::new(
                    point(
                        bounds.left(),
                        bounds.top() + px(layout.top + f32::from(THEME.space.sm)),
                    ),
                    size(bounds.size.width, THEME.border),
                ),
                THEME.colors.border,
            ));
        }
        let expanded = view.expanded_transcript_items.contains(&layout.index);
        let lines = shape_item(item, expanded, bounds.size.width, window);
        let body_x = bounds.left()
            + THEME.space.md
            + if item.kind == TranscriptKind::Tool {
                THEME.layout.transcript_label_width + THEME.space.sm
            } else {
                px(0.0)
            };
        let label = if item.kind == TranscriptKind::Tool && !item.label.is_empty() {
            let line = shape_label(item, window);
            let label_bounds = Bounds::new(
                point(
                    bounds.left() + THEME.space.md,
                    bounds.top() + px(layout.content_top),
                ),
                size(
                    THEME.layout.transcript_label_width,
                    THEME.type_scale.line_body,
                ),
            );
            Some(LabelPaint {
                line,
                origin: label_bounds.origin,
                bounds: label_bounds,
            })
        } else {
            None
        };
        state.items.push(ItemPaint {
            origin: point(body_x, bounds.top() + px(layout.content_top)),
            lines,
            label,
        });
    }
    state
}

fn shape_item(
    item: &TranscriptItem,
    expanded: bool,
    total_width: Pixels,
    window: &mut Window,
) -> Vec<WrappedLine> {
    let body_width = (total_width
        - THEME.space.md * 2.0
        - if item.kind == TranscriptKind::Tool {
            THEME.layout.transcript_label_width + THEME.space.sm
        } else {
            px(0.0)
        })
    .max(px(1.0));
    let text = display_text(item, expanded);
    let font = item_font(item, window);
    let color = item_color(item);
    if item.kind == TranscriptKind::Tool {
        shape_limited_text(
            text.as_str(),
            TOOL_VISUAL_LINES,
            &font,
            color,
            body_width,
            window,
        )
    } else {
        shape_text(text, &font, color, body_width, None, window)
    }
}

fn shape_limited_text(
    text: &str,
    max_rows: usize,
    font: &Font,
    color: Hsla,
    width: Pixels,
    window: &mut Window,
) -> Vec<WrappedLine> {
    let mut rows = 0;
    let mut shaped = Vec::new();
    for line in text.lines() {
        if rows >= max_rows {
            break;
        }
        let remaining = max_rows - rows;
        let mut next = shape_text(
            SharedString::from(line),
            font,
            color,
            width,
            Some(remaining),
            window,
        );
        rows += next.iter().map(visual_rows).sum::<usize>();
        shaped.append(&mut next);
    }
    if shaped.is_empty() {
        shape_text("…".into(), font, color, width, Some(1), window)
    } else {
        shaped
    }
}

fn shape_text(
    text: SharedString,
    font: &Font,
    color: Hsla,
    width: Pixels,
    clamp: Option<usize>,
    window: &mut Window,
) -> Vec<WrappedLine> {
    let run = TextRun {
        len: text.len(),
        font: font.clone(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_text(
            text,
            THEME.type_scale.body_small,
            &[run],
            Some(width),
            clamp,
        )
        .map_or_else(|_| Vec::new(), |lines| lines.into_iter().collect())
}

fn shape_label(item: &TranscriptItem, window: &mut Window) -> WrappedLine {
    let text = SharedString::from(item.label.as_str());
    let mut font = window.text_style().font();
    font.weight = FontWeight::MEDIUM;
    let run = TextRun {
        len: text.len(),
        font,
        color: label_color(item),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_text(
            text,
            THEME.type_scale.caption,
            &[run],
            Some(THEME.layout.transcript_label_width),
            Some(1),
        )
        .ok()
        .and_then(|mut lines| lines.pop())
        .unwrap_or_default()
}

fn display_text(item: &TranscriptItem, expanded: bool) -> SharedString {
    if item.kind == TranscriptKind::Tool {
        return SharedString::from(tool_preview(&item.text));
    }
    if item.kind == TranscriptKind::Thinking && !expanded {
        return SharedString::from(item.text.lines().next().unwrap_or("…"));
    }
    if item.text.is_empty() {
        "…".into()
    } else {
        SharedString::from(item.text.as_str())
    }
}

fn tool_preview(text: &str) -> String {
    let mut preview = String::new();
    let mut truncated = false;
    for (index, line) in text.lines().enumerate() {
        if index >= TOOL_VISUAL_LINES {
            truncated = true;
            break;
        }
        if index > 0 {
            preview.push('\n');
        }
        let remaining = TOOL_PREVIEW_CHARS.saturating_sub(preview.chars().count());
        if remaining == 0 {
            truncated = true;
            break;
        }
        let line_chars = line.chars().count();
        preview.extend(line.chars().take(remaining));
        if line_chars > remaining {
            truncated = true;
            break;
        }
    }
    if text.is_empty() {
        return "…".into();
    }
    if truncated {
        preview.push('…');
    }
    preview
}

fn item_font(item: &TranscriptItem, window: &Window) -> Font {
    let mut font = if item.kind == TranscriptKind::Tool {
        gpui::font("monospace")
    } else {
        window.text_style().font()
    };
    if item.kind == TranscriptKind::User {
        font.weight = FontWeight::MEDIUM;
    }
    if item.kind == TranscriptKind::Thinking {
        font.style = FontStyle::Italic;
    }
    font
}

fn item_color(item: &TranscriptItem) -> Hsla {
    match item.kind {
        TranscriptKind::User | TranscriptKind::Assistant => THEME.colors.text.into(),
        TranscriptKind::Thinking => THEME.colors.subtle.into(),
        TranscriptKind::Tool if item.is_error => THEME.colors.error.into(),
        TranscriptKind::Tool => THEME.colors.muted.into(),
        TranscriptKind::Error => THEME.colors.error.into(),
        TranscriptKind::Notice | TranscriptKind::Custom => THEME.colors.muted.into(),
    }
}

fn label_color(item: &TranscriptItem) -> Hsla {
    if item.is_error {
        THEME.colors.error.into()
    } else if item.streaming {
        THEME.colors.warning.into()
    } else {
        THEME.colors.subtle.into()
    }
}

fn visual_rows(line: &WrappedLine) -> usize {
    line.wrap_boundaries().len() + 1
}

fn starts_transcript_chunk(items: &[TranscriptItem], index: usize) -> bool {
    index > 0
        && items.get(index).is_some_and(|item| {
            item.kind == TranscriptKind::User || items[index - 1].kind == TranscriptKind::User
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: TranscriptKind, text: &str) -> TranscriptItem {
        TranscriptItem {
            kind,
            label: String::new(),
            text: text.into(),
            streaming: false,
            is_error: false,
        }
    }

    #[test]
    fn chunks_split_only_at_speaker_turns() {
        let items = vec![
            item(TranscriptKind::User, "question"),
            item(TranscriptKind::Thinking, "plan"),
            item(TranscriptKind::Tool, "args"),
            item(TranscriptKind::Assistant, "answer"),
            item(TranscriptKind::User, "next"),
            item(TranscriptKind::Assistant, "reply"),
        ];
        assert_eq!(
            (0..items.len())
                .map(|index| starts_transcript_chunk(&items, index))
                .collect::<Vec<_>>(),
            [false, true, false, false, true, true]
        );
    }

    #[test]
    fn tool_preview_is_bounded_before_layout() {
        let text = format!("first\nsecond\nthird\n{}", "x".repeat(1_000));
        assert_eq!(tool_preview(&text), "first\nsecond\nthird…");
        assert!(tool_preview(&"x".repeat(1_000)).chars().count() <= TOOL_PREVIEW_CHARS + 1);
    }
}
