//! Transcript-owned virtual viewport with frame-coalesced scrolling.
//!
//! Durable state contains only row measurements and logical scroll position. GPUI elements stay
//! frame-local: measured overdraw warms the height cache, while visible rows are rebuilt for
//! prepaint and paint. Row splices, row remeasurement, and width changes are the cache-invalidation
//! boundary.

use std::{cell::RefCell, collections::BTreeMap, ops::Range, rc::Rc};

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, ContentMask, DispatchPhase, Element, ElementId,
    EntityId, FollowMode, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement,
    LayoutId, Pixels, Point, ScrollWheelEvent, Size, Style, Window, point, px, relative,
};

const DEFAULT_ROW_HEIGHT: Pixels = px(24.0);

type RenderRow = dyn FnMut(usize, &mut Window, &mut App) -> AnyElement + 'static;
type ScrollHandler = dyn FnMut(&TranscriptScrollEvent, &mut Window, &mut App) + 'static;

#[derive(Clone, Copy, Debug)]
pub(crate) struct TranscriptScrollEvent {
    pub(crate) is_following_tail: bool,
}

#[derive(Clone, Copy, Debug)]
struct RowHeight {
    value: Pixels,
    measured: bool,
}

impl RowHeight {
    fn estimated(value: Pixels) -> Self {
        Self {
            value: value.max(px(1.0)),
            measured: false,
        }
    }
}

#[derive(Default)]
struct HeightIndex {
    rows: Vec<RowHeight>,
    fenwick: Vec<f32>,
}

impl HeightIndex {
    fn from_rows(rows: Vec<RowHeight>) -> Self {
        let mut index = Self {
            fenwick: Vec::new(),
            rows,
        };
        index.rebuild();
        index
    }

    fn rebuild(&mut self) {
        self.fenwick = vec![0.0; self.rows.len() + 1];
        for (row, height) in self.rows.iter().enumerate() {
            let slot = row + 1;
            self.fenwick[slot] += f32::from(height.value);
            let parent = slot + (slot & slot.wrapping_neg());
            if parent < self.fenwick.len() {
                self.fenwick[parent] += self.fenwick[slot];
            }
        }
    }

    fn add(&mut self, row: usize, delta: f32) {
        let mut slot = row + 1;
        while slot < self.fenwick.len() {
            self.fenwick[slot] += delta;
            slot += slot & slot.wrapping_neg();
        }
    }

    fn extend(&mut self, rows: impl IntoIterator<Item = RowHeight>) {
        for row in rows {
            let slot = self.rows.len() + 1;
            let block_start = slot - (slot & slot.wrapping_neg());
            let preceding = self.prefix(slot - 1) - self.prefix(block_start);
            self.rows.push(row);
            self.fenwick
                .push(f32::from(preceding + self.rows[slot - 1].value));
        }
    }

    fn set_height(&mut self, row: usize, height: Pixels) {
        let height = height.max(px(1.0));
        let previous = self.rows[row].value;
        self.rows[row].value = height;
        self.rows[row].measured = true;
        self.add(row, f32::from(height - previous));
    }

    fn prefix(&self, end: usize) -> Pixels {
        let mut slot = end.min(self.rows.len());
        let mut height = 0.0;
        while slot > 0 {
            height += self.fenwick[slot];
            slot &= slot - 1;
        }
        px(height)
    }

    fn total(&self) -> Pixels {
        self.prefix(self.rows.len())
    }

    /// Return the row containing `offset`, or `len` when offset is at the end.
    fn row_at(&self, offset: Pixels) -> usize {
        let target = f32::from(offset.max(px(0.0)));
        let mut index = 0;
        let mut prefix = 0.0;
        let mut step = self.rows.len().next_power_of_two();
        while step > 0 {
            let next = index + step;
            if next < self.fenwick.len() && prefix + self.fenwick[next] <= target {
                index = next;
                prefix += self.fenwick[next];
            }
            step >>= 1;
        }
        index.min(self.rows.len())
    }
}

struct StateInner {
    rows: HeightIndex,
    scroll_y: Pixels,
    viewport_height: Pixels,
    last_width: Option<Pixels>,
    tail_mode: bool,
    following_tail: bool,
    pending_delta: Point<Pixels>,
    next_frame_token: u64,
    scheduled_frame: Option<u64>,
    scroll_handler: Option<Rc<RefCell<Box<ScrollHandler>>>>,
}

#[derive(Clone)]
pub(crate) struct TranscriptListState(Rc<RefCell<StateInner>>);

impl std::fmt::Debug for TranscriptListState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("TranscriptListState")
    }
}

impl Default for TranscriptListState {
    fn default() -> Self {
        Self::new()
    }
}

impl TranscriptListState {
    pub(crate) fn new() -> Self {
        Self(Rc::new(RefCell::new(StateInner {
            rows: HeightIndex::default(),
            scroll_y: px(0.0),
            viewport_height: px(0.0),
            last_width: None,
            tail_mode: false,
            following_tail: false,
            pending_delta: point(px(0.0), px(0.0)),
            next_frame_token: 0,
            scheduled_frame: None,
            scroll_handler: None,
        })))
    }

    pub(crate) fn reset(&self, row_count: usize) {
        let mut state = self.0.borrow_mut();
        state.rows =
            HeightIndex::from_rows(vec![RowHeight::estimated(DEFAULT_ROW_HEIGHT); row_count]);
        state.scroll_y = px(0.0);
        state.pending_delta = point(px(0.0), px(0.0));
    }

    #[cfg(test)]
    pub(crate) fn splice(&self, old_range: Range<usize>, count: usize) {
        self.splice_with_size_hints(old_range, std::iter::repeat_n(DEFAULT_ROW_HEIGHT, count));
    }

    pub(crate) fn splice_with_size_hints(
        &self,
        old_range: Range<usize>,
        size_hints: impl IntoIterator<Item = Pixels>,
    ) {
        let mut state = self.0.borrow_mut();
        let was_empty = state.rows.rows.is_empty();
        let anchor = logical_scroll_top(&state);
        let old_len = old_range.end.saturating_sub(old_range.start);
        let replacement = size_hints
            .into_iter()
            .map(RowHeight::estimated)
            .collect::<Vec<_>>();
        let replacement_len = replacement.len();
        let appends_to_existing = !was_empty
            && old_range.start == state.rows.rows.len()
            && old_range.end == state.rows.rows.len();
        if appends_to_existing {
            state.rows.extend(replacement);
        } else {
            state.rows.rows.splice(old_range.clone(), replacement);
            state.rows.rebuild();
        }

        let mut anchor_index = anchor.item_ix;
        let mut anchor_offset = anchor.offset_in_item;
        if was_empty {
            anchor_index = 0;
            anchor_offset = px(0.0);
        } else if old_range.contains(&anchor_index) {
            anchor_index = old_range.start.min(state.rows.rows.len());
            anchor_offset = px(0.0);
        } else if old_range.end <= anchor_index {
            anchor_index = anchor_index
                .saturating_sub(old_len)
                .saturating_add(replacement_len)
                .min(state.rows.rows.len());
        }
        state.scroll_y = state.rows.prefix(anchor_index) + anchor_offset;
        clamp_scroll(&mut state);
    }

    pub(crate) fn remeasure_items(&self, range: Range<usize>) {
        let mut state = self.0.borrow_mut();
        for row in state.rows.rows.get_mut(range).into_iter().flatten() {
            row.measured = false;
        }
    }

    pub(crate) fn logical_scroll_top(&self) -> gpui::ListOffset {
        logical_scroll_top(&self.0.borrow())
    }

    #[cfg(test)]
    pub(crate) fn scroll_by(&self, distance: Pixels) {
        let mut state = self.0.borrow_mut();
        state.scroll_y += distance;
        if distance < px(0.0) {
            state.following_tail = false;
        }
        clamp_scroll(&mut state);
    }

    pub(crate) fn scroll_to(&self, offset: gpui::ListOffset) {
        let mut state = self.0.borrow_mut();
        let index = offset.item_ix.min(state.rows.rows.len());
        state.scroll_y = state.rows.prefix(index) + offset.offset_in_item;
        if index < state.rows.rows.len() {
            state.following_tail = false;
        }
        clamp_scroll(&mut state);
    }

    pub(crate) fn scroll_to_end(&self) {
        let mut state = self.0.borrow_mut();
        state.pending_delta = point(px(0.0), px(0.0));
        state.following_tail = true;
        state.scroll_y = maximum_scroll(&state);
    }

    pub(crate) fn set_follow_mode(&self, mode: FollowMode) {
        let mut state = self.0.borrow_mut();
        match mode {
            FollowMode::Normal => {
                state.tail_mode = false;
                state.following_tail = false;
            }
            FollowMode::Tail => {
                state.pending_delta = point(px(0.0), px(0.0));
                state.tail_mode = true;
                state.following_tail = true;
                state.scroll_y = maximum_scroll(&state);
            }
        }
    }

    pub(crate) fn pause_following_tail(&self) {
        self.0.borrow_mut().following_tail = false;
    }

    pub(crate) fn is_following_tail(&self) -> bool {
        let state = self.0.borrow();
        state.tail_mode && state.following_tail
    }

    pub(crate) fn set_scroll_handler(
        &self,
        handler: impl FnMut(&TranscriptScrollEvent, &mut Window, &mut App) + 'static,
    ) {
        self.0.borrow_mut().scroll_handler = Some(Rc::new(RefCell::new(Box::new(handler))));
    }

    fn queue_scroll(&self, delta: Point<Pixels>) -> Option<u64> {
        if delta.y == px(0.0) {
            return None;
        }
        let mut state = self.0.borrow_mut();
        state.pending_delta += delta;
        let maximum = maximum_scroll(&state);
        let projected = (state.scroll_y - state.pending_delta.y)
            .max(px(0.0))
            .min(maximum);
        let resumes_tail = state.tail_mode
            && !state.following_tail
            && state.pending_delta.y < px(0.0)
            && state.scroll_y >= maximum - px(1.0);
        if projected == state.scroll_y && !resumes_tail {
            state.pending_delta = point(px(0.0), px(0.0));
            return None;
        }
        if state.scheduled_frame.is_some() {
            None
        } else {
            state.next_frame_token = state.next_frame_token.wrapping_add(1);
            let token = state.next_frame_token;
            state.scheduled_frame = Some(token);
            Some(token)
        }
    }

    #[cfg(test)]
    pub(crate) fn pending_scroll_delta(&self) -> Point<Pixels> {
        self.0.borrow().pending_delta
    }
}

pub(crate) fn transcript_list(
    state: TranscriptListState,
    render_row: impl FnMut(usize, &mut Window, &mut App) -> AnyElement + 'static,
) -> TranscriptList {
    TranscriptList {
        state,
        render_row: Box::new(render_row),
    }
}

pub(crate) struct TranscriptList {
    state: TranscriptListState,
    render_row: Box<RenderRow>,
}

impl IntoElement for TranscriptList {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

struct MeasuredRow {
    index: usize,
    element: AnyElement,
    size: Size<Pixels>,
}

struct RowLayout {
    element: AnyElement,
}

pub(crate) struct TranscriptPrepaintState {
    hitbox: Hitbox,
    rows: Vec<RowLayout>,
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
            state.viewport_height = bounds.size.height;
            clamp_scroll(&mut state);
            let width_changed = state
                .last_width
                .is_none_or(|last_width| f32::from(last_width - bounds.size.width).abs() >= 0.5);
            if width_changed {
                state.last_width = Some(bounds.size.width);
                for row in &mut state.rows.rows {
                    row.measured = false;
                }
            }

            let pending = std::mem::replace(&mut state.pending_delta, point(px(0.0), px(0.0)));
            state.scheduled_frame = None;
            let scrolled = pending.y != px(0.0);
            if scrolled {
                state.scroll_y = (state.scroll_y - pending.y).max(px(0.0));
                if pending.y > px(0.0) {
                    state.following_tail = false;
                }
                clamp_scroll(&mut state);
            }
            if state.following_tail {
                state.scroll_y = maximum_scroll(&state);
            }

            (
                logical_scroll_top(&state),
                state.scroll_handler.clone(),
                scrolled,
            )
        };

        let available = gpui::size(bounds.size.width.into(), AvailableSpace::MinContent);
        let overdraw = crate::theme::THEME.layout.transcript_overdraw;
        let mut measured = BTreeMap::new();

        // Measurements can change the visible range. Iterate until every visible row has a
        // frame-local element and every invalidated row in the overdraw has refreshed its cache.
        let (scroll_y, following_tail, visible_range) = loop {
            let needed = {
                let state = self.state.0.borrow();
                let visible = layout_range(&state, px(0.0));
                layout_range(&state, overdraw)
                    .filter(|index| {
                        (visible.contains(index) || !state.rows.rows[*index].measured)
                            && !measured.contains_key(index)
                    })
                    .collect::<Vec<_>>()
            };

            if !needed.is_empty() {
                let rows = measure_rows(self.render_row.as_mut(), needed, available, window, cx);
                let mut state = self.state.0.borrow_mut();
                for row in rows {
                    if row.index < state.rows.rows.len() {
                        state.rows.set_height(row.index, row.size.height);
                    }
                    measured.insert(row.index, row);
                }
                restore_anchor(&mut state, anchor);
                if state.following_tail {
                    state.scroll_y = maximum_scroll(&state);
                }
                continue;
            }

            let mut state = self.state.0.borrow_mut();
            if state.tail_mode
                && !state.following_tail
                && state.scroll_y >= maximum_scroll(&state) - px(1.0)
            {
                state.following_tail = true;
                state.scroll_y = maximum_scroll(&state);
            }
            let visible = layout_range(&state, px(0.0));
            if visible.clone().any(|index| !measured.contains_key(&index)) {
                continue;
            }
            break (
                state.scroll_y,
                state.tail_mode && state.following_tail,
                visible,
            );
        };

        let mut rows = Vec::with_capacity(visible_range.len());
        let mut row_y =
            bounds.top() + self.state.0.borrow().rows.prefix(visible_range.start) - scroll_y;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for index in visible_range {
                let mut row = measured
                    .remove(&index)
                    .expect("visible transcript rows must be laid out");
                row.element
                    .prepaint_at(point(bounds.left(), row_y), window, cx);
                row_y += row.size.height;
                rows.push(RowLayout {
                    element: row.element,
                });
            }
        });

        if scrolled && let Some(handler) = handler {
            handler.borrow_mut()(
                &TranscriptScrollEvent {
                    is_following_tail: following_tail,
                },
                window,
                cx,
            );
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
            crate::performance::record_scroll_event(event.touch_phase);
            let delta = event.delta.pixel_delta(px(20.0));
            if let Some(token) = state.queue_scroll(delta) {
                request_scroll_frame(window, current_view, state.clone(), token);
            }
        });

        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for row in &mut prepaint.rows {
                row.element.paint(window, cx);
            }
        });
    }
}

fn measure_rows(
    render_row: &mut RenderRow,
    indices: impl IntoIterator<Item = usize>,
    available: Size<AvailableSpace>,
    window: &mut Window,
    cx: &mut App,
) -> Vec<MeasuredRow> {
    indices
        .into_iter()
        .map(|index| {
            let mut element = render_row(index, window, cx);
            let mut size = element.layout_as_root(available, window, cx);
            size.height = size.height.max(px(1.0));
            MeasuredRow {
                index,
                element,
                size,
            }
        })
        .collect()
}

fn request_scroll_frame(window: &Window, view: EntityId, state: TranscriptListState, token: u64) {
    window.on_next_frame(move |_, cx| {
        let should_notify = {
            let mut state = state.0.borrow_mut();
            if state.scheduled_frame != Some(token) {
                false
            } else if state.pending_delta.y == px(0.0) {
                state.scheduled_frame = None;
                false
            } else {
                true
            }
        };
        if should_notify {
            cx.notify(view);
        }
    });
}

fn restore_anchor(state: &mut StateInner, anchor: gpui::ListOffset) {
    let item_ix = anchor.item_ix.min(state.rows.rows.len());
    let offset = state.rows.rows.get(item_ix).map_or(px(0.0), |row| {
        anchor.offset_in_item.max(px(0.0)).min(row.value)
    });
    state.scroll_y = state.rows.prefix(item_ix) + offset;
    clamp_scroll(state);
}

fn maximum_scroll(state: &StateInner) -> Pixels {
    (state.rows.total() - state.viewport_height).max(px(0.0))
}

fn clamp_scroll(state: &mut StateInner) {
    state.scroll_y = state.scroll_y.max(px(0.0)).min(maximum_scroll(state));
}

fn logical_scroll_top(state: &StateInner) -> gpui::ListOffset {
    let item_ix = state.rows.row_at(state.scroll_y);
    gpui::ListOffset {
        item_ix,
        offset_in_item: state.scroll_y - state.rows.prefix(item_ix),
    }
}

fn layout_range(state: &StateInner, overdraw: Pixels) -> Range<usize> {
    let start_y = (state.scroll_y - overdraw).max(px(0.0));
    let end_y = state.scroll_y + state.viewport_height + overdraw;
    let start = state.rows.row_at(start_y);
    let end = if end_y >= state.rows.total() {
        state.rows.rows.len()
    } else {
        state.rows.row_at(end_y).saturating_add(1)
    };
    start.min(end)..end.min(state.rows.rows.len())
}

#[cfg(test)]
#[path = "transcript_list_tests.rs"]
mod tests;
