//! Dense diff rendering through one viewport-aware GPUI element.

use std::{ops::Range, rc::Rc};

use gpui::{
    App, Bounds, ContentMask, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, PaintQuad, Pixels, Point, Rgba, Style, TextAlign, TextStyle, Window, WrappedLine,
    fill, point, px, relative,
};

use crate::{
    syntax_highlight::HighlightedText,
    theme::{MONO_FONT_FAMILY, THEME},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DiffTone {
    Context,
    Addition,
    Deletion,
    Muted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DiffCell {
    pub(crate) gutter: Option<String>,
    pub(crate) text: HighlightedText,
    pub(crate) tone: DiffTone,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct DiffPaintRow {
    pub(crate) old: Option<DiffCell>,
    pub(crate) new: Option<DiffCell>,
}

type RowProvider = Rc<dyn Fn(usize) -> DiffPaintRow>;

pub(crate) struct DiffElement {
    row_count: usize,
    split: bool,
    line_height: Pixels,
    gutter_width: Pixels,
    row_provider: RowProvider,
}

impl DiffElement {
    pub(crate) fn unified(
        row_count: usize,
        line_height: Pixels,
        gutter_width: Pixels,
        row: impl Fn(usize) -> Option<DiffCell> + 'static,
    ) -> Self {
        Self {
            row_count,
            split: false,
            line_height,
            gutter_width,
            row_provider: Rc::new(move |index| DiffPaintRow {
                old: None,
                new: row(index),
            }),
        }
    }

    pub(crate) fn split(
        row_count: usize,
        line_height: Pixels,
        gutter_width: Pixels,
        row: impl Fn(usize) -> DiffPaintRow + 'static,
    ) -> Self {
        Self {
            row_count,
            split: true,
            line_height,
            gutter_width,
            row_provider: Rc::new(row),
        }
    }
}

impl IntoElement for DiffElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

struct TextPaint {
    line: WrappedLine,
    origin: Point<Pixels>,
    line_height: Pixels,
    align: TextAlign,
    bounds: Bounds<Pixels>,
    mask: ContentMask<Pixels>,
}

#[derive(Default)]
pub(crate) struct DiffPrepaintState {
    quads: Vec<PaintQuad>,
    text: Vec<TextPaint>,
}

impl Element for DiffElement {
    type RequestLayoutState = ();
    type PrepaintState = DiffPrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = (self.line_height * self.row_count as f32).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let visible = visible_row_range(
            self.row_count,
            self.line_height,
            bounds,
            window.content_mask().bounds,
        );
        let _timing = crate::performance::OperationTiming::new(
            crate::performance::OperationKind::DiffPrepaint,
            visible.len(),
        );
        let mut state = DiffPrepaintState::default();
        let base_style = diff_text_style(window);

        for index in visible {
            let row = (self.row_provider)(index);
            let top = bounds.top() + self.line_height * index as f32;
            let row_bounds = Bounds::new(
                point(bounds.left(), top),
                gpui::size(bounds.size.width, self.line_height),
            );
            if self.split {
                let old_width = row_bounds.size.width * 0.5;
                let old_bounds =
                    Bounds::new(row_bounds.origin, gpui::size(old_width, self.line_height));
                let new_bounds = Bounds::new(
                    point(row_bounds.left() + old_width, top),
                    gpui::size(
                        (row_bounds.size.width - old_width).max(px(0.0)),
                        self.line_height,
                    ),
                );
                prepare_cell(
                    row.old.as_ref(),
                    old_bounds,
                    self.gutter_width,
                    &base_style,
                    window,
                    &mut state,
                );
                prepare_cell(
                    row.new.as_ref(),
                    new_bounds,
                    self.gutter_width,
                    &base_style,
                    window,
                    &mut state,
                );
                state.quads.push(fill(
                    Bounds::new(
                        point(new_bounds.left(), top),
                        gpui::size(THEME.border, self.line_height),
                    ),
                    THEME.colors.border,
                ));
            } else {
                prepare_cell(
                    row.new.as_ref().or(row.old.as_ref()),
                    row_bounds,
                    self.gutter_width,
                    &base_style,
                    window,
                    &mut state,
                );
            }
        }
        state
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let _timing = crate::performance::Timing::new(if self.split {
            "diff.paint_split"
        } else {
            "diff.paint_unified"
        });
        for quad in prepaint.quads.drain(..) {
            window.paint_quad(quad);
        }
        for text in prepaint.text.drain(..) {
            window.with_content_mask(Some(text.mask), |window| {
                let _ = text.line.paint(
                    text.origin,
                    text.line_height,
                    text.align,
                    Some(text.bounds),
                    window,
                    cx,
                );
            });
        }
    }
}

fn diff_text_style(window: &Window) -> TextStyle {
    let mut style = window.text_style();
    style.font_family = MONO_FONT_FAMILY.into();
    style.font_size = THEME.type_scale.body_small.into();
    style.line_height = THEME.type_scale.line_body.into();
    style
}

fn prepare_cell(
    cell: Option<&DiffCell>,
    bounds: Bounds<Pixels>,
    gutter_width: Pixels,
    base_style: &TextStyle,
    window: &mut Window,
    state: &mut DiffPrepaintState,
) {
    let (background, foreground) = cell
        .map(|cell| tone_colors(cell.tone))
        .unwrap_or((THEME.colors.canvas, THEME.colors.text));
    state.quads.push(fill(bounds, background));
    let Some(cell) = cell else {
        return;
    };

    let gutter_width = gutter_width.min(bounds.size.width);
    if gutter_width > px(0.0) {
        let gutter_bounds =
            Bounds::new(bounds.origin, gpui::size(gutter_width, bounds.size.height));
        if let Some(gutter) = cell.gutter.as_deref().filter(|gutter| !gutter.is_empty()) {
            let mut style = base_style.clone();
            style.color = if cell.tone == DiffTone::Context {
                THEME.colors.subtle.into()
            } else {
                foreground.into()
            };
            if let Some(line) = shape_line(gutter.into(), vec![style.to_run(gutter.len())], window)
            {
                let text_bounds = inset_horizontal(gutter_bounds, THEME.space.xs);
                state.text.push(TextPaint {
                    line,
                    origin: text_bounds.origin,
                    line_height: bounds.size.height,
                    align: TextAlign::Right,
                    bounds: text_bounds,
                    mask: ContentMask {
                        bounds: gutter_bounds,
                    },
                });
            }
        }
    }

    let content_bounds = Bounds::new(
        point(bounds.left() + gutter_width, bounds.top()),
        gpui::size(
            (bounds.size.width - gutter_width).max(px(0.0)),
            bounds.size.height,
        ),
    );
    if content_bounds.size.width <= px(0.0) {
        return;
    }
    let mut style = base_style.clone();
    style.color = foreground.into();
    let text = cell.text.shared_text();
    if let Some(line) = shape_line(text.clone(), cell.text.runs(&style), window) {
        let text_bounds = inset_horizontal(content_bounds, THEME.space.xs);
        state.text.push(TextPaint {
            line,
            origin: text_bounds.origin,
            line_height: bounds.size.height,
            align: TextAlign::Left,
            bounds: text_bounds,
            mask: ContentMask {
                bounds: content_bounds,
            },
        });
    }
}

fn shape_line(
    text: gpui::SharedString,
    runs: Vec<gpui::TextRun>,
    window: &Window,
) -> Option<WrappedLine> {
    if text.is_empty() {
        return None;
    }
    window
        .text_system()
        .shape_text(text, THEME.type_scale.body_small, &runs, None, Some(1))
        .ok()
        .and_then(|mut lines| lines.pop())
}

fn inset_horizontal(bounds: Bounds<Pixels>, inset: Pixels) -> Bounds<Pixels> {
    let inset = inset.min(bounds.size.width * 0.5);
    Bounds::new(
        point(bounds.left() + inset, bounds.top()),
        gpui::size(
            (bounds.size.width - inset * 2.0).max(px(0.0)),
            bounds.size.height,
        ),
    )
}

fn tone_colors(tone: DiffTone) -> (Rgba, Rgba) {
    match tone {
        DiffTone::Context => (THEME.colors.canvas, THEME.colors.text),
        DiffTone::Addition => (THEME.colors.diff_added, THEME.colors.success),
        DiffTone::Deletion => (THEME.colors.diff_deleted, THEME.colors.error),
        DiffTone::Muted => (THEME.colors.surface, THEME.colors.subtle),
    }
}

fn visible_row_range(
    row_count: usize,
    line_height: Pixels,
    bounds: Bounds<Pixels>,
    clip: Bounds<Pixels>,
) -> Range<usize> {
    if row_count == 0 || line_height <= px(0.0) {
        return 0..0;
    }
    let visible = bounds.intersect(&clip);
    if visible.size.height <= px(0.0) || visible.size.width <= px(0.0) {
        return 0..0;
    }
    let line_height = f32::from(line_height);
    let top = (f32::from(visible.top() - bounds.top()) / line_height).floor() as usize;
    let bottom = (f32::from(visible.bottom() - bounds.top()) / line_height).ceil() as usize;
    top.saturating_sub(2)..bottom.saturating_add(2).min(row_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_rows_are_bounded_to_the_clipped_viewport() {
        let bounds = Bounds::new(
            point(px(0.0), px(-200.0)),
            gpui::size(px(400.0), px(2_000.0)),
        );
        let clip = Bounds::new(point(px(0.0), px(0.0)), gpui::size(px(400.0), px(100.0)));

        assert_eq!(visible_row_range(100, px(20.0), bounds, clip), 8..17);
    }

    #[test]
    fn offscreen_diff_prepares_no_rows() {
        let bounds = Bounds::new(point(px(0.0), px(500.0)), gpui::size(px(400.0), px(200.0)));
        let clip = Bounds::new(point(px(0.0), px(0.0)), gpui::size(px(400.0), px(100.0)));

        assert_eq!(visible_row_range(10, px(20.0), bounds, clip), 0..0);
    }
}
