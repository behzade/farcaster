use std::{cell::RefCell, collections::BTreeMap, ops::Range, rc::Rc};

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, ContentMask, DispatchPhase, Element, ElementId,
    EntityId, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId,
    ListOffset, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    ScrollWheelEvent, Size, Style, Window, point, px, relative,
};

#[path = "list/height_index.rs"]
mod height_index;
use self::height_index::HeightIndex;

type RenderRow = dyn FnMut(usize, &mut Window, &mut App) -> AnyElement + 'static;
type ScrollHandler = dyn FnMut(bool, &mut Window, &mut App) + 'static;
type SelectionText = dyn Fn(std::ops::RangeInclusive<usize>) -> String + 'static;

pub(crate) const TRANSCRIPT_SELECTION_KEY_CONTEXT: &str = "PiTranscriptSelection";

#[derive(Default)]
struct StateInner {
    heights: HeightIndex,
    scroll_y: Pixels,
    viewport_height: Pixels,
    width: Option<Pixels>,
    following_tail: bool,
    pending_scroll: Pixels,
    next_frame_token: u64,
    scheduled_frame: Option<u64>,
    selection_drag_active: bool,
    selection_anchor_candidate: Option<usize>,
    selection_anchor: Option<usize>,
    selection_cursor: Option<usize>,
    selection_drag_position: Option<gpui::Point<Pixels>>,
    selection_scroll_delta: Option<Pixels>,
    selection_text: Option<String>,
    scroll_handler: Option<Rc<RefCell<Box<ScrollHandler>>>>,
}

impl StateInner {
    fn maximum_scroll(&self) -> Pixels {
        (self.heights.total() - self.viewport_height).max(px(0.0))
    }

    fn clamp_scroll(&mut self) {
        self.scroll_y = self.scroll_y.max(px(0.0)).min(self.maximum_scroll());
    }

    fn logical_scroll_top(&self) -> ListOffset {
        let item_ix = self.heights.row_at(self.scroll_y);
        ListOffset {
            item_ix,
            offset_in_item: self.scroll_y - self.heights.prefix(item_ix),
        }
    }

    fn restore_anchor(&mut self, anchor: ListOffset) {
        let item_ix = anchor.item_ix.min(self.heights.len());
        let offset = self.heights.height(item_ix).map_or(px(0.0), |height| {
            anchor.offset_in_item.max(px(0.0)).min(height)
        });
        self.scroll_y = self.heights.prefix(item_ix) + offset;
        self.clamp_scroll();
    }

    fn layout_range(&self, overdraw: Pixels) -> Range<usize> {
        let start_y = (self.scroll_y - overdraw).max(px(0.0));
        let end_y = self.scroll_y + self.viewport_height + overdraw;
        let start = self.heights.row_at(start_y);
        let end = if end_y >= self.heights.total() {
            self.heights.len()
        } else {
            self.heights.row_at(end_y).saturating_add(1)
        };
        start.min(end)..end.min(self.heights.len())
    }

    fn begin_frame(&mut self, size: Size<Pixels>) -> (ListOffset, bool) {
        self.viewport_height = size.height;
        self.clamp_scroll();
        if self
            .width
            .is_none_or(|width| f32::from(width - size.width).abs() >= 0.5)
        {
            self.width = Some(size.width);
            self.heights.invalidate_all();
        }

        let pending = std::mem::replace(&mut self.pending_scroll, px(0.0));
        self.scheduled_frame = None;
        let scrolled = pending != px(0.0);
        if scrolled {
            self.scroll_y -= pending;
            if pending > px(0.0) {
                self.following_tail = false;
            }
            self.clamp_scroll();
        }
        if self.following_tail {
            self.scroll_y = self.maximum_scroll();
        }
        (self.logical_scroll_top(), scrolled)
    }

    fn resume_tail_at_end(&mut self) {
        if !self.following_tail && self.scroll_y >= self.maximum_scroll() - px(1.0) {
            self.following_tail = true;
            self.scroll_y = self.maximum_scroll();
        }
    }

    fn queue_scroll(&mut self, delta: Pixels) -> Option<u64> {
        if delta == px(0.0) {
            return None;
        }
        self.pending_scroll += delta;
        let maximum = self.maximum_scroll();
        let projected = (self.scroll_y - self.pending_scroll)
            .max(px(0.0))
            .min(maximum);
        let resumes_tail = !self.following_tail
            && self.pending_scroll < px(0.0)
            && self.scroll_y >= maximum - px(1.0);
        if projected == self.scroll_y && !resumes_tail {
            self.pending_scroll = px(0.0);
            return None;
        }
        if self.scheduled_frame.is_some() {
            return None;
        }
        self.next_frame_token = self.next_frame_token.wrapping_add(1);
        self.scheduled_frame = Some(self.next_frame_token);
        self.scheduled_frame
    }

    fn should_notify(&mut self, token: u64) -> bool {
        if self.scheduled_frame != Some(token) {
            return false;
        }
        if self.pending_scroll == px(0.0) {
            self.scheduled_frame = None;
            return false;
        }
        true
    }

    fn row_at_viewport_y(&self, y: Pixels) -> Option<usize> {
        (!self.heights.is_empty()).then(|| {
            self.heights
                .row_at((self.scroll_y + y).max(px(0.0)))
                .min(self.heights.len() - 1)
        })
    }

    fn confirm_selection_drag(&mut self, has_selection: bool, cursor: Option<usize>) -> bool {
        if self.selection_anchor.is_none() && self.selection_drag_active && has_selection {
            self.selection_anchor = self.selection_anchor_candidate;
        }
        if self.selection_anchor.is_some() {
            self.selection_cursor = cursor;
        }
        self.selection_anchor.is_some()
    }

    fn selection_range(&self) -> Option<std::ops::RangeInclusive<usize>> {
        let (Some(anchor), Some(cursor)) = (self.selection_anchor, self.selection_cursor) else {
            return None;
        };
        (anchor != cursor).then_some(anchor.min(cursor)..=anchor.max(cursor))
    }

    fn selection_contains(&self, key: usize) -> bool {
        self.selection_range()
            .is_some_and(|range| range.contains(&key))
    }

    fn clear_selection(&mut self) {
        self.selection_drag_active = false;
        self.selection_anchor_candidate = None;
        self.selection_anchor = None;
        self.selection_cursor = None;
        self.selection_drag_position = None;
        self.selection_scroll_delta = None;
        self.selection_text = None;
    }

    fn set_selection_scroll(&mut self, delta: Option<Pixels>) -> Option<u64> {
        self.selection_scroll_delta = delta;
        delta.and_then(|delta| self.queue_scroll(-delta))
    }

    fn continue_selection_scroll(&mut self) -> Option<u64> {
        self.selection_scroll_delta
            .and_then(|delta| self.queue_scroll(-delta))
    }
}

#[derive(Clone)]
pub(crate) struct TranscriptListState(Rc<RefCell<StateInner>>);

impl TranscriptListState {
    pub(crate) fn new() -> Self {
        Self(Rc::new(RefCell::new(StateInner::default())))
    }

    pub(crate) fn reset(&self) {
        let mut state = self.0.borrow_mut();
        state.heights = HeightIndex::default();
        state.scroll_y = px(0.0);
        state.pending_scroll = px(0.0);
        state.following_tail = false;
        state.clear_selection();
    }

    pub(crate) fn splice_with_size_hints(
        &self,
        old_range: Range<usize>,
        size_hints: impl IntoIterator<Item = Pixels>,
    ) {
        let mut state = self.0.borrow_mut();
        let was_empty = state.heights.is_empty();
        let anchor = state.logical_scroll_top();
        let old_len = old_range.len();
        let replacement_len = state.heights.splice(old_range.clone(), size_hints);

        let (anchor_index, anchor_offset) = if was_empty {
            (0, px(0.0))
        } else if old_range.contains(&anchor.item_ix) {
            (old_range.start.min(state.heights.len()), px(0.0))
        } else if old_range.end <= anchor.item_ix {
            (
                anchor
                    .item_ix
                    .saturating_sub(old_len)
                    .saturating_add(replacement_len)
                    .min(state.heights.len()),
                anchor.offset_in_item,
            )
        } else {
            (anchor.item_ix, anchor.offset_in_item)
        };
        state.scroll_y = state.heights.prefix(anchor_index) + anchor_offset;
        state.clamp_scroll();
    }

    pub(crate) fn remeasure_items(&self, range: Range<usize>) {
        self.0.borrow_mut().heights.invalidate(range);
    }

    pub(crate) fn logical_scroll_top(&self) -> ListOffset {
        self.0.borrow().logical_scroll_top()
    }

    pub(crate) fn scroll_to(&self, offset: ListOffset) {
        let mut state = self.0.borrow_mut();
        let index = offset.item_ix.min(state.heights.len());
        state.scroll_y = state.heights.prefix(index) + offset.offset_in_item;
        if index < state.heights.len() {
            state.following_tail = false;
        }
        state.clamp_scroll();
    }

    pub(crate) fn scroll_to_end(&self) {
        let mut state = self.0.borrow_mut();
        state.pending_scroll = px(0.0);
        state.following_tail = true;
        state.scroll_y = state.maximum_scroll();
    }

    pub(crate) fn pause_following_tail(&self) {
        self.0.borrow_mut().following_tail = false;
    }

    pub(crate) fn is_following_tail(&self) -> bool {
        self.0.borrow().following_tail
    }

    pub(crate) fn set_scroll_handler(
        &self,
        handler: impl FnMut(bool, &mut Window, &mut App) + 'static,
    ) {
        self.0.borrow_mut().scroll_handler = Some(Rc::new(RefCell::new(Box::new(handler))));
    }

    pub(crate) fn selection_contains(&self, key: usize) -> bool {
        self.0.borrow().selection_contains(key)
    }

    pub(crate) fn selected_text(&self) -> Option<String> {
        self.0.borrow().selection_text.clone()
    }

    fn queue_scroll(&self, delta: Pixels) -> Option<u64> {
        self.0.borrow_mut().queue_scroll(delta)
    }
}

pub(crate) fn transcript_list_grouped(
    state: TranscriptListState,
    selection_key: impl Fn(usize) -> usize + 'static,
    selection_text: impl Fn(std::ops::RangeInclusive<usize>) -> String + 'static,
    render_row: impl FnMut(usize, &mut Window, &mut App) -> AnyElement + 'static,
) -> TranscriptList {
    TranscriptList {
        state,
        selection_key: Rc::new(selection_key),
        selection_text: Rc::new(selection_text),
        render_row: Box::new(render_row),
    }
}

pub(crate) struct TranscriptList {
    state: TranscriptListState,
    selection_key: Rc<dyn Fn(usize) -> usize>,
    selection_text: Rc<SelectionText>,
    render_row: Box<RenderRow>,
}

impl IntoElement for TranscriptList {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

struct FrameRow {
    index: usize,
    element: AnyElement,
    height: Pixels,
}

pub(crate) struct TranscriptPrepaintState {
    hitbox: Hitbox,
    rows: Vec<AnyElement>,
}

impl Element for TranscriptList {
    type RequestLayoutState = ();
    type PrepaintState = TranscriptPrepaintState;

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
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let (anchor, handler, scrolled) = {
            let mut state = self.state.0.borrow_mut();
            let (anchor, scrolled) = state.begin_frame(bounds.size);
            if state.selection_drag_active
                && state.selection_anchor.is_some()
                && let Some(position) = state.selection_drag_position
                && let Some(row) = state.row_at_viewport_y(position.y - bounds.top())
            {
                state.selection_cursor = Some((self.selection_key)(row));
            }
            (anchor, state.scroll_handler.clone(), scrolled)
        };

        let available = gpui::size(bounds.size.width.into(), AvailableSpace::MinContent);
        let overdraw = crate::app::ui::theme::THEME.layout.transcript_overdraw;
        let mut frame_rows = BTreeMap::new();

        let (scroll_y, following_tail, visible_range) = loop {
            let needed = {
                let state = self.state.0.borrow();
                let visible = state.layout_range(px(0.0));
                state
                    .layout_range(overdraw)
                    .filter(|index| {
                        !frame_rows.contains_key(index)
                            && (visible.contains(index) || !state.heights.is_measured(*index))
                    })
                    .collect::<Vec<_>>()
            };

            if !needed.is_empty() {
                let rows = needed
                    .into_iter()
                    .map(|index| {
                        let mut element = (self.render_row)(index, window, cx);
                        let height = element
                            .layout_as_root(available, window, cx)
                            .height
                            .max(px(1.0));
                        FrameRow {
                            index,
                            element,
                            height,
                        }
                    })
                    .collect::<Vec<_>>();
                let mut state = self.state.0.borrow_mut();
                for row in rows {
                    if row.index < state.heights.len() {
                        state.heights.set_height(row.index, row.height);
                    }
                    frame_rows.insert(row.index, row);
                }
                state.restore_anchor(anchor);
                if state.following_tail {
                    state.scroll_y = state.maximum_scroll();
                }
                continue;
            }

            let mut state = self.state.0.borrow_mut();
            state.resume_tail_at_end();
            let visible = state.layout_range(px(0.0));
            if visible.clone().all(|index| frame_rows.contains_key(&index)) {
                break (state.scroll_y, state.following_tail, visible);
            }
        };

        let mut rows = Vec::with_capacity(visible_range.len());
        let mut row_y =
            bounds.top() + self.state.0.borrow().heights.prefix(visible_range.start) - scroll_y;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for index in visible_range {
                let mut row = frame_rows
                    .remove(&index)
                    .expect("visible transcript rows must be laid out");
                row.element
                    .prepaint_at(point(bounds.left(), row_y), window, cx);
                row_y += row.height;
                rows.push(row.element);
            }
        });

        if scrolled && let Some(handler) = handler {
            handler.borrow_mut()(following_tail, window, cx);
        }

        TranscriptPrepaintState { hitbox, rows }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let current_view = window.current_view();
        let hitbox_id = prepaint.hitbox.id;
        let state = self.state.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, _cx| {
            if phase != DispatchPhase::Bubble || !hitbox_id.should_handle_scroll(window) {
                return;
            }
            crate::app::performance::record_scroll_event(event.touch_phase);
            if let Some(token) = state.queue_scroll(event.delta.pixel_delta(px(20.0)).y) {
                request_scroll_frame(window, current_view, state.clone(), token);
            }
        });

        let selection_state = self.state.clone();
        let selection_key = self.selection_key.clone();
        let selection_hitbox_id = prepaint.hitbox.id;
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                let mut state = selection_state.0.borrow_mut();
                let had_selection = state.selection_anchor.is_some();
                let inside = selection_hitbox_id.is_hovered(window);
                state.selection_drag_active = inside;
                state.selection_anchor_candidate = inside
                    .then(|| state.row_at_viewport_y(event.position.y - bounds.top()))
                    .flatten()
                    .map(|row| selection_key(row));
                state.selection_anchor = None;
                state.selection_cursor = None;
                state.selection_text = None;
                state.selection_drag_position = inside.then_some(event.position);
                if had_selection {
                    cx.notify(current_view);
                }
            }
        });

        let selection_state = self.state.clone();
        let selection_key = self.selection_key.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            if !selection_state.0.borrow().selection_drag_active
                || event.pressed_button != Some(MouseButton::Left)
            {
                selection_state.0.borrow_mut().set_selection_scroll(None);
                return;
            }
            let (selection_drag_confirmed, selection_changed) = {
                let mut state = selection_state.0.borrow_mut();
                state.selection_drag_position = Some(event.position);
                let cursor = state
                    .row_at_viewport_y(event.position.y - bounds.top())
                    .map(|row| selection_key(row));
                let previous = state.selection_cursor;
                let confirmed = state.confirm_selection_drag(
                    gpui_base::TextSelection::has_selection(window, cx),
                    cursor,
                );
                (confirmed, previous != state.selection_cursor)
            };
            if selection_changed {
                cx.notify(current_view);
            }
            let delta = (selection_drag_confirmed
                && event.pressed_button == Some(MouseButton::Left))
            .then(|| gpui_base::AutoScroll::compute_delta(event.position.y, bounds))
            .flatten();
            if let Some(token) = selection_state.0.borrow_mut().set_selection_scroll(delta) {
                request_scroll_frame(window, current_view, selection_state.clone(), token);
            }
        });

        let selection_state = self.state.clone();
        let selection_text = self.selection_text.clone();
        window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let range = {
                let mut state = selection_state.0.borrow_mut();
                state.selection_drag_active = false;
                state.selection_anchor_candidate = None;
                state.selection_drag_position = None;
                state.set_selection_scroll(None);
                state.selection_range()
            };
            if let Some(range) = range {
                let text = selection_text(range);
                gpui_base::TextSelection::clear(window, cx);
                selection_state.0.borrow_mut().selection_text = Some(text);
                cx.notify(current_view);
            }
        });

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for row in &mut prepaint.rows {
                row.paint(window, cx);
            }
        });

        if let Some(token) = self.state.0.borrow_mut().continue_selection_scroll() {
            request_scroll_frame(window, current_view, self.state.clone(), token);
        }
    }
}

fn request_scroll_frame(window: &Window, view: EntityId, state: TranscriptListState, token: u64) {
    window.on_next_frame(move |_, cx| {
        if state.0.borrow_mut().should_notify(token) {
            cx.notify(view);
        }
    });
}

#[cfg(test)]
#[path = "list/tests.rs"]
mod tests;
