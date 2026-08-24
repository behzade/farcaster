use super::*;
use gpui::{
    AppContext as _, Context, ParentElement as _, Render, ScrollDelta, Styled as _, TestAppContext,
    VisualTestContext, div, size,
};
use gpui_base::TextSelectionLayer;

struct FixedHeightView {
    state: TranscriptListState,
    row_height: Pixels,
    rendered: Rc<RefCell<Vec<usize>>>,
}

struct CopyableRowsView {
    state: TranscriptListState,
}

impl Render for CopyableRowsView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(TextSelectionLayer)
            .child(transcript_list_grouped(
                self.state.clone(),
                |index| index,
                |range| {
                    range
                        .map(|index| index.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                },
                |_, _, _| div().h(px(24.)).w_full().into_any_element(),
            ))
    }
}

impl Render for FixedHeightView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let rendered = self.rendered.clone();
        let row_height = self.row_height;
        transcript_list_grouped(
            self.state.clone(),
            |index| index,
            |_| String::new(),
            move |index, _, _| {
                rendered.borrow_mut().push(index);
                div().h(row_height).w_full().into_any_element()
            },
        )
    }
}

fn state_with_rows(count: usize) -> TranscriptListState {
    let state = TranscriptListState::new();
    state.splice_with_size_hints(0..0, std::iter::repeat_n(px(24.0), count));
    state
}

fn draw_transcript(
    cx: &mut VisualTestContext,
    state: &TranscriptListState,
    row_height: Pixels,
    viewport_height: Pixels,
) -> Vec<usize> {
    let rendered = Rc::new(RefCell::new(Vec::new()));
    cx.draw(
        point(px(0.0), px(0.0)),
        size(px(100.0), viewport_height),
        |_, cx| {
            cx.new(|_| FixedHeightView {
                state: state.clone(),
                row_height,
                rendered: rendered.clone(),
            })
            .into_any_element()
        },
    );
    let rows = rendered.borrow().clone();
    rows
}

fn wheel(cx: &mut VisualTestContext, delta: Pixels) {
    cx.simulate_event(ScrollWheelEvent {
        position: point(px(1.0), px(1.0)),
        delta: ScrollDelta::Pixels(point(px(0.0), delta)),
        ..Default::default()
    });
}

#[test]
fn height_hints_locate_rows_across_append() {
    let state = TranscriptListState::new();
    state.splice_with_size_hints(0..0, [px(10.0), px(20.0)]);
    state.splice_with_size_hints(2..2, [px(30.0), px(40.0)]);

    let heights = &state.0.borrow().heights;
    assert_eq!(heights.row_at(px(15.0)), 1);
    assert_eq!(heights.row_at(px(65.0)), 3);
    assert_eq!(heights.total(), px(100.0));
}

#[test]
fn jump_to_end_supersedes_queued_wheel_input() {
    let state = state_with_rows(100);
    state.0.borrow_mut().viewport_height = px(100.0);
    state.scroll_to_end();

    assert!(state.queue_scroll(px(12.0)).is_some());
    state.scroll_to_end();

    let inner = state.0.borrow();
    assert_eq!(inner.pending_scroll, px(0.0));
    assert!(inner.following_tail);
    assert_eq!(inner.scroll_y, inner.maximum_scroll());
}

#[gpui::test]
fn wheel_events_coalesce_until_the_next_layout(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let state = state_with_rows(100);
    state.scroll_to(ListOffset {
        item_ix: 10,
        offset_in_item: px(0.0),
    });
    draw_transcript(cx, &state, px(24.0), px(100.0));

    wheel(cx, px(12.0));
    wheel(cx, px(8.0));
    assert_eq!(state.0.borrow().pending_scroll, px(20.0));
    assert_eq!(state.logical_scroll_top().item_ix, 10);
    assert_eq!(cx.update(|window, cx| window.simulate_next_frame(cx)), 1);

    draw_transcript(cx, &state, px(24.0), px(100.0));
    let offset = state.logical_scroll_top();
    assert_eq!(offset.item_ix, 9);
    assert_eq!(offset.offset_in_item, px(4.0));
    assert_eq!(cx.update(|window, cx| window.simulate_next_frame(cx)), 0);
}

#[test]
fn confirmed_selection_drag_survives_a_virtualization_gap() {
    let state = TranscriptListState::new();
    let mut inner = state.0.borrow_mut();
    inner.selection_drag_active = true;
    inner.selection_anchor_candidate = Some(2);

    assert!(inner.confirm_selection_drag(true, Some(4)));
    assert!(inner.confirm_selection_drag(false, Some(8)));
    assert_eq!(inner.selection_anchor, Some(2));
    assert_eq!(inner.selection_cursor, Some(8));
    assert!(inner.selection_contains(6));
}

#[test]
fn cross_element_selection_marks_the_inclusive_logical_range() {
    let state = TranscriptListState::new();
    let mut inner = state.0.borrow_mut();
    inner.selection_anchor = Some(7);
    inner.selection_cursor = Some(3);

    assert!(!inner.selection_contains(2));
    assert!(inner.selection_contains(3));
    assert!(inner.selection_contains(5));
    assert!(inner.selection_contains(7));
    assert!(!inner.selection_contains(8));

    inner.selection_cursor = Some(7);
    assert!(!inner.selection_contains(7));
}

#[gpui::test]
fn coarse_selection_is_exposed_to_window_copy(cx: &mut TestAppContext) {
    let state = state_with_rows(10);
    let (_, cx) = cx.add_window_view({
        let state = state.clone();
        move |_, _| CopyableRowsView { state }
    });
    let cx: &mut VisualTestContext = cx;
    cx.update(|window, cx| {
        let _ = window.draw(cx);
    });

    let handle = {
        let mut inner = state.0.borrow_mut();
        inner.selection_anchor = Some(1);
        inner.selection_cursor = Some(3);
        inner
            .selection_handle
            .clone()
            .expect("drawing creates the stable transcript selection handle")
    };
    cx.update(|window, cx| {
        handle.set_local_selection(true, cx);
        let _ = window.draw(cx);
    });

    let copied = cx.update(|window, cx| gpui_base::TextSelection::selected_text(window, cx));
    assert_eq!(copied, "1,2,3");
}

#[test]
fn selection_edge_scroll_repeats_until_stopped() {
    let state = state_with_rows(100);
    let mut inner = state.0.borrow_mut();
    inner.viewport_height = px(100.0);
    inner.scroll_y = px(240.0);

    let first = inner
        .set_selection_scroll(Some(px(12.0)))
        .expect("first edge drag schedules a frame");
    assert!(inner.should_notify(first));
    assert_eq!(inner.begin_frame(size(px(100.), px(100.))).1, true);
    assert_eq!(inner.scroll_y, px(252.0));

    let second = inner
        .continue_selection_scroll()
        .expect("active edge drag schedules another frame");
    assert!(inner.should_notify(second));
    assert_eq!(inner.begin_frame(size(px(100.), px(100.))).1, true);
    assert_eq!(inner.scroll_y, px(264.0));

    inner.set_selection_scroll(None);
    assert!(inner.continue_selection_scroll().is_none());
}

#[test]
fn selection_edge_scroll_stops_at_the_document_boundary() {
    let state = state_with_rows(10);
    let mut inner = state.0.borrow_mut();
    inner.viewport_height = px(100.0);
    inner.scroll_y = inner.maximum_scroll();

    let final_frame = inner
        .set_selection_scroll(Some(px(12.0)))
        .expect("the final downward frame resumes tail following");
    assert!(inner.should_notify(final_frame));
    inner.begin_frame(size(px(100.), px(100.)));
    inner.resume_tail_at_end();
    assert!(inner.continue_selection_scroll().is_none());
    assert_eq!(inner.pending_scroll, px(0.0));
}

#[gpui::test]
fn measurement_shrink_fills_the_viewport_in_one_layout(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let state = TranscriptListState::new();
    state.splice_with_size_hints(0..0, std::iter::repeat_n(px(100.0), 100));

    let rendered = draw_transcript(cx, &state, px(10.0), px(100.0));

    assert!(rendered.into_iter().max().unwrap_or_default() >= 9);
}

#[gpui::test]
fn measured_overdraw_is_cached_until_invalidated(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let state = state_with_rows(100);

    let cold_rows = draw_transcript(cx, &state, px(24.0), px(100.0));
    let cached_rows = draw_transcript(cx, &state, px(24.0), px(100.0));
    assert!(cached_rows.len() < cold_rows.len());
    assert!(!cached_rows.contains(&8));

    state.remeasure_items(8..9);
    let invalidated_rows = draw_transcript(cx, &state, px(24.0), px(100.0));
    assert!(invalidated_rows.contains(&8));
}

#[gpui::test]
fn growing_viewport_clamps_before_selecting_rows(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let state = state_with_rows(100);
    state.scroll_to(ListOffset {
        item_ix: 95,
        offset_in_item: px(0.0),
    });

    draw_transcript(cx, &state, px(24.0), px(100.0));
    let rendered = draw_transcript(cx, &state, px(24.0), px(1_000.0));

    let top = state.logical_scroll_top().item_ix;
    let first_rendered = rendered
        .into_iter()
        .min()
        .expect("the viewport should contain transcript rows");
    assert!(first_rendered <= top);
}

#[gpui::test]
fn remeasurement_clamps_an_anchor_to_the_new_row_height(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let state = TranscriptListState::new();
    state.splice_with_size_hints(
        0..0,
        std::iter::once(px(1_000.0)).chain(std::iter::repeat_n(px(20.0), 99)),
    );
    state.scroll_to(ListOffset {
        item_ix: 0,
        offset_in_item: px(500.0),
    });

    draw_transcript(cx, &state, px(20.0), px(100.0));

    let offset = state.logical_scroll_top();
    assert_eq!(offset.item_ix, 1);
    assert_eq!(offset.offset_in_item, px(0.0));
}

#[gpui::test]
fn downward_scroll_at_the_end_resumes_tail_following(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let state = state_with_rows(100);
    state.scroll_to_end();
    draw_transcript(cx, &state, px(24.0), px(100.0));
    state.pause_following_tail();

    wheel(cx, px(-10.0));
    assert_eq!(cx.update(|window, cx| window.simulate_next_frame(cx)), 1);
    draw_transcript(cx, &state, px(24.0), px(100.0));

    assert!(state.is_following_tail());
}

#[gpui::test]
fn tail_resume_uses_final_measured_heights(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let state = TranscriptListState::new();
    state.splice_with_size_hints(0..0, std::iter::repeat_n(px(20.0), 10));
    state.scroll_to_end();
    draw_transcript(cx, &state, px(20.0), px(100.0));
    state.pause_following_tail();

    assert!(state.queue_scroll(px(-10.0)).is_some());
    draw_transcript(cx, &state, px(100.0), px(100.0));

    assert!(!state.is_following_tail());
    let inner = state.0.borrow();
    assert!(inner.scroll_y < inner.maximum_scroll());
}

#[test]
fn splice_preserves_anchor_after_rows_before_it_change() {
    let state = TranscriptListState::new();
    state.splice_with_size_hints(0..0, [px(20.0), px(20.0), px(20.0)]);
    state.scroll_to(ListOffset {
        item_ix: 2,
        offset_in_item: px(5.0),
    });
    state.splice_with_size_hints(0..1, [px(10.0), px(10.0)]);

    let offset = state.logical_scroll_top();
    assert_eq!(offset.item_ix, 3);
    assert_eq!(offset.offset_in_item, px(5.0));
}
