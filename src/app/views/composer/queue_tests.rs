use super::*;
use gpui::{Context, Render, px};

struct QueueHarness {
    app: WeakEntity<FarcasterApp>,
    queue: QueueState,
    receipts: Vec<PendingReceipt>,
    target: String,
    width: f32,
}

impl Render for QueueHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(self.width)).children(render(
            &self.queue,
            &self.receipts,
            &self.target,
            Some(std::path::Path::new("/test/session")),
            self.app.clone(),
            true,
            false,
        ))
    }
}

#[gpui::test]
fn queue_groups_keep_expansion_warnings_and_cancel_permissions(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::queue_groups_keep_expansion_warnings_and_cancel_permissions"
        ),
        cx,
        |cx, app, _, _| {
            let view = cx.update(|window, cx| {
                window.replace_root(cx, |_, _| QueueHarness {
                    app: app.downgrade(),
                    queue: QueueState {
                        steering: vec!["check this first".into(), "uncertain message".into()],
                        steering_ids: vec!["steer".into(), "unknown".into()],
                        follow_up: vec!["then test\nincluding the empty state".into()],
                        follow_up_ids: vec!["later".into()],
                        cancellable_ids: vec!["steer".into()],
                        ..Default::default()
                    },
                    receipts: vec![PendingReceipt {
                        id: "unknown".into(),
                        mode: Some(PromptMode::Steer),
                        text: "uncertain message".into(),
                        images: Default::default(),
                        unknown: true,
                    }],
                    target: "test".into(),
                    width: 220.0,
                })
            });
            for width in [220.0, 480.0] {
                cx.update(|window, cx| {
                    view.update(cx, |view, cx| {
                        view.width = width;
                        cx.notify();
                    });
                    window.draw(cx).clear(cx);
                });
                let steer = "queue-row:test:Steer:steer";
                let follow_up = "queue-row:test:FollowUp:later";
                assert!(cx.debug_bounds("queue-group-Steer").is_some());
                assert!(cx.debug_bounds("queue-group-FollowUp").is_some());
                assert!(cx.debug_bounds(steer).is_some());
                let preview = cx.debug_bounds(follow_up).expect("follow-up row");
                assert!(preview.right() <= px(width));
                assert!(preview.size.height <= theme().layout.status_row_height);
                assert!(cx.debug_bounds("queue-delivery-warning").is_some());
                assert!(cx.debug_bounds("cancel-queue-steer").is_some());
                assert!(cx.debug_bounds("cancel-queue-unknown").is_none());
                assert!(cx.debug_bounds("cancel-queue-later").is_none());

                let preview = cx
                    .debug_bounds("queue-row:test:FollowUp:later-preview")
                    .expect("follow-up preview");
                cx.simulate_click(preview.center(), Default::default());
                cx.update(|window, cx| window.draw(cx).clear(cx));
                let body = cx
                    .debug_bounds("queue-message-body")
                    .expect("expanded body");
                assert!(body.right() <= px(width));
                cx.simulate_click(preview.center(), Default::default());
                cx.update(|window, cx| window.draw(cx).clear(cx));
            }
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.queue.steering.clear();
                    view.queue.steering_ids.clear();
                    view.receipts.clear();
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            assert!(cx.debug_bounds("queue-row:test:FollowUp:later").is_some());
            assert!(cx.debug_bounds("queue-group-Steer").is_none());
            assert!(cx.debug_bounds("queue-group-FollowUp").is_some());
            assert!(cx.debug_bounds("queue-delivery-warning").is_none());
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.queue = QueueState::default();
                    cx.notify();
                });
                window.draw(cx).clear(cx);
            });
            assert!(cx.debug_bounds("composer-queue-tray").is_none());
        },
    );
}
