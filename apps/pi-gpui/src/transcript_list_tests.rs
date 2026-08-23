use super::*;
use gpui::{AppContext as _, Context, Render, ScrollDelta, Styled as _, TestAppContext, div, size};

struct FixedHeightView {
    state: TranscriptListState,
    row_height: Pixels,
    rendered: Rc<RefCell<Vec<usize>>>,
}

impl Render for FixedHeightView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let rendered = self.rendered.clone();
        let row_height = self.row_height;
        transcript_list(self.state.clone(), move |index, _, _| {
            rendered.borrow_mut().push(index);
            div().h(row_height).w_full().into_any_element()
        })
    }
}

#[test]
fn size_hints_locate_scroll_anchor_without_rendering() {
    let state = TranscriptListState::new();
    state.splice_with_size_hints(0..0, [px(20.0), px(80.0), px(40.0)]);
    state.scroll_by(px(30.0));

    let offset = state.logical_scroll_top();
    assert_eq!(offset.item_ix, 1);
    assert_eq!(offset.offset_in_item, px(10.0));
}

#[test]
fn appended_height_hints_extend_the_index() {
    let state = TranscriptListState::new();
    state.splice_with_size_hints(0..0, [px(10.0), px(20.0)]);
    state.splice_with_size_hints(2..2, [px(30.0), px(40.0)]);
    state.scroll_by(px(65.0));

    let offset = state.logical_scroll_top();
    assert_eq!(offset.item_ix, 3);
    assert_eq!(offset.offset_in_item, px(5.0));
}

#[test]
fn scroll_events_accumulate_and_schedule_one_frame() {
    let state = TranscriptListState::new();
    state.splice(0..0, 100);
    state.scroll_to(gpui::ListOffset {
        item_ix: 10,
        offset_in_item: px(0.0),
    });

    assert!(state.queue_scroll(point(px(0.0), px(12.0))).is_some());
    assert!(state.queue_scroll(point(px(0.0), px(8.0))).is_none());
    assert_eq!(state.pending_scroll_delta().y, px(20.0));
}

#[test]
fn jump_to_end_supersedes_queued_wheel_input() {
    let state = TranscriptListState::new();
    state.splice(0..0, 100);
    state.0.borrow_mut().viewport_height = px(100.0);
    state.set_follow_mode(FollowMode::Tail);

    assert!(state.queue_scroll(point(px(0.0), px(12.0))).is_some());
    state.scroll_to_end();

    let inner = state.0.borrow();
    assert_eq!(inner.pending_delta.y, px(0.0));
    assert!(inner.following_tail);
    assert_eq!(inner.scroll_y, maximum_scroll(&inner));
}

#[test]
fn opposite_scroll_events_cancel_before_layout() {
    let state = TranscriptListState::new();
    state.splice(0..0, 100);
    state.scroll_to(gpui::ListOffset {
        item_ix: 10,
        offset_in_item: px(0.0),
    });

    assert!(state.queue_scroll(point(px(0.0), px(12.0))).is_some());
    assert!(state.queue_scroll(point(px(0.0), px(-12.0))).is_none());
    assert_eq!(state.pending_scroll_delta().y, px(0.0));
}

#[gpui::test]
fn cancelled_wheel_batch_skips_view_invalidation(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let state = TranscriptListState::new();
    state.splice(0..0, 100);
    state.scroll_to(gpui::ListOffset {
        item_ix: 10,
        offset_in_item: px(0.0),
    });
    let rendered = Rc::new(RefCell::new(Vec::new()));
    cx.draw(
        point(px(0.0), px(0.0)),
        size(px(100.0), px(100.0)),
        |_, cx| {
            cx.new(|_| FixedHeightView {
                state: state.clone(),
                row_height: px(24.0),
                rendered: rendered.clone(),
            })
            .into_any_element()
        },
    );

    for delta in [px(12.0), px(-12.0)] {
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(1.0), px(1.0)),
            delta: ScrollDelta::Pixels(point(px(0.0), delta)),
            ..Default::default()
        });
    }
    assert_eq!(state.pending_scroll_delta().y, px(0.0));
    assert!(state.0.borrow().scheduled_frame.is_some());
    assert_eq!(cx.update(|window, cx| window.simulate_next_frame(cx)), 1);
    assert!(state.0.borrow().scheduled_frame.is_none());
    assert_eq!(state.logical_scroll_top().item_ix, 10);
}

#[gpui::test]
fn element_coalesces_wheel_events_until_the_next_layout(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let state = TranscriptListState::new();
    state.splice(0..0, 100);
    state.scroll_to(gpui::ListOffset {
        item_ix: 10,
        offset_in_item: px(0.0),
    });
    let rendered = Rc::new(RefCell::new(Vec::new()));
    cx.draw(
        point(px(0.0), px(0.0)),
        size(px(100.0), px(100.0)),
        |_, cx| {
            cx.new(|_| FixedHeightView {
                state: state.clone(),
                row_height: px(24.0),
                rendered: rendered.clone(),
            })
            .into_any_element()
        },
    );

    for delta in [px(12.0), px(8.0)] {
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(1.0), px(1.0)),
            delta: ScrollDelta::Pixels(point(px(0.0), delta)),
            ..Default::default()
        });
    }
    assert_eq!(state.pending_scroll_delta().y, px(20.0));
    assert_eq!(state.logical_scroll_top().item_ix, 10);
    assert_eq!(cx.update(|window, cx| window.simulate_next_frame(cx)), 1);

    cx.draw(
        point(px(0.0), px(0.0)),
        size(px(100.0), px(100.0)),
        |_, cx| {
            cx.new(|_| FixedHeightView {
                state: state.clone(),
                row_height: px(24.0),
                rendered: rendered.clone(),
            })
            .into_any_element()
        },
    );
    let offset = state.logical_scroll_top();
    assert_eq!(offset.item_ix, 9);
    assert_eq!(offset.offset_in_item, px(4.0));
    assert_eq!(cx.update(|window, cx| window.simulate_next_frame(cx)), 0);
}

#[gpui::test]
fn measurement_shrink_fills_the_viewport_in_one_layout(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let state = TranscriptListState::new();
    state.splice_with_size_hints(0..0, std::iter::repeat_n(px(100.0), 100));
    let rendered = Rc::new(RefCell::new(Vec::new()));

    cx.draw(
        point(px(0.0), px(0.0)),
        size(px(100.0), px(100.0)),
        |_, cx| {
            cx.new(|_| FixedHeightView {
                state: state.clone(),
                row_height: px(10.0),
                rendered: rendered.clone(),
            })
            .into_any_element()
        },
    );

    assert!(rendered.borrow().iter().copied().max().unwrap_or_default() >= 9);
}

#[gpui::test]
fn measured_overdraw_is_cached_until_invalidated(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let state = TranscriptListState::new();
    state.splice(0..0, 100);
    let rendered = Rc::new(RefCell::new(Vec::new()));
    let draw = |cx: &mut gpui::VisualTestContext, rendered: &Rc<RefCell<Vec<usize>>>| {
        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(100.0), px(100.0)),
            |_, cx| {
                cx.new(|_| FixedHeightView {
                    state: state.clone(),
                    row_height: px(24.0),
                    rendered: rendered.clone(),
                })
                .into_any_element()
            },
        );
    };

    draw(cx, &rendered);
    let cold_rows = rendered.borrow().len();
    rendered.borrow_mut().clear();
    draw(cx, &rendered);
    let cached_rows = rendered.borrow().len();
    assert!(cached_rows < cold_rows);
    assert!(!rendered.borrow().contains(&8));

    state.remeasure_items(8..9);
    rendered.borrow_mut().clear();
    draw(cx, &rendered);
    assert!(rendered.borrow().contains(&8));
}

#[gpui::test]
fn growing_viewport_clamps_before_selecting_rows(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let state = TranscriptListState::new();
    state.splice(0..0, 100);
    state.scroll_to(gpui::ListOffset {
        item_ix: 95,
        offset_in_item: px(0.0),
    });
    let rendered = Rc::new(RefCell::new(Vec::new()));

    for height in [px(100.0), px(1_000.0)] {
        rendered.borrow_mut().clear();
        cx.draw(point(px(0.0), px(0.0)), size(px(100.0), height), |_, cx| {
            cx.new(|_| FixedHeightView {
                state: state.clone(),
                row_height: px(24.0),
                rendered: rendered.clone(),
            })
            .into_any_element()
        });
    }

    let top = state.logical_scroll_top().item_ix;
    assert!(rendered.borrow().iter().copied().min().unwrap_or(top) <= top);
}

#[gpui::test]
fn remeasurement_clamps_an_anchor_to_the_new_row_height(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let state = TranscriptListState::new();
    state.splice_with_size_hints(
        0..0,
        std::iter::once(px(1_000.0)).chain(std::iter::repeat_n(px(20.0), 99)),
    );
    state.scroll_to(gpui::ListOffset {
        item_ix: 0,
        offset_in_item: px(500.0),
    });
    let rendered = Rc::new(RefCell::new(Vec::new()));

    cx.draw(
        point(px(0.0), px(0.0)),
        size(px(100.0), px(100.0)),
        |_, cx| {
            cx.new(|_| FixedHeightView {
                state: state.clone(),
                row_height: px(20.0),
                rendered: rendered.clone(),
            })
            .into_any_element()
        },
    );

    let offset = state.logical_scroll_top();
    assert_eq!(offset.item_ix, 1);
    assert_eq!(offset.offset_in_item, px(0.0));
}

#[gpui::test]
fn downward_scroll_at_the_end_resumes_tail_following(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let state = TranscriptListState::new();
    state.splice(0..0, 100);
    state.set_follow_mode(FollowMode::Tail);
    let rendered = Rc::new(RefCell::new(Vec::new()));
    cx.draw(
        point(px(0.0), px(0.0)),
        size(px(100.0), px(100.0)),
        |_, cx| {
            cx.new(|_| FixedHeightView {
                state: state.clone(),
                row_height: px(24.0),
                rendered: rendered.clone(),
            })
            .into_any_element()
        },
    );
    state.pause_following_tail();

    cx.simulate_event(ScrollWheelEvent {
        position: point(px(1.0), px(1.0)),
        delta: ScrollDelta::Pixels(point(px(0.0), px(-10.0))),
        ..Default::default()
    });
    assert_eq!(cx.update(|window, cx| window.simulate_next_frame(cx)), 1);
    cx.draw(
        point(px(0.0), px(0.0)),
        size(px(100.0), px(100.0)),
        |_, cx| {
            cx.new(|_| FixedHeightView {
                state: state.clone(),
                row_height: px(24.0),
                rendered: rendered.clone(),
            })
            .into_any_element()
        },
    );

    assert!(state.is_following_tail());
}

#[gpui::test]
fn tail_resume_uses_final_measured_heights(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let state = TranscriptListState::new();
    state.splice_with_size_hints(0..0, std::iter::repeat_n(px(20.0), 10));
    state.set_follow_mode(FollowMode::Tail);
    let rendered = Rc::new(RefCell::new(Vec::new()));
    cx.draw(
        point(px(0.0), px(0.0)),
        size(px(100.0), px(100.0)),
        |_, cx| {
            cx.new(|_| FixedHeightView {
                state: state.clone(),
                row_height: px(20.0),
                rendered: rendered.clone(),
            })
            .into_any_element()
        },
    );
    state.pause_following_tail();

    assert!(state.queue_scroll(point(px(0.0), px(-10.0))).is_some());
    cx.draw(
        point(px(0.0), px(0.0)),
        size(px(100.0), px(100.0)),
        |_, cx| {
            cx.new(|_| FixedHeightView {
                state: state.clone(),
                row_height: px(100.0),
                rendered: rendered.clone(),
            })
            .into_any_element()
        },
    );

    assert!(!state.is_following_tail());
    let inner = state.0.borrow();
    assert!(inner.scroll_y < maximum_scroll(&inner));
}

#[test]
fn splice_preserves_anchor_after_rows_before_it_change() {
    let state = TranscriptListState::new();
    state.splice_with_size_hints(0..0, [px(20.0), px(20.0), px(20.0)]);
    state.scroll_to(gpui::ListOffset {
        item_ix: 2,
        offset_in_item: px(5.0),
    });
    state.splice_with_size_hints(0..1, [px(10.0), px(10.0)]);

    let offset = state.logical_scroll_top();
    assert_eq!(offset.item_ix, 3);
    assert_eq!(offset.offset_in_item, px(5.0));
}
