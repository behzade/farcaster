use gpui::{
    Context, Entity, ParentElement as _, Render, Styled as _, Subscription, WeakEntity, div,
};

use super::super::{FarcasterApp, workgraph::WorkGraphBoardView};
use crate::app::ui::{
    primitives::{ButtonTone, button},
    theme::theme,
};

pub(crate) struct WorkGraphDetailView {
    app: WeakEntity<FarcasterApp>,
    board: Entity<WorkGraphBoardView>,
    _board_subscription: Subscription,
}

impl WorkGraphDetailView {
    pub(crate) fn new(
        app: WeakEntity<FarcasterApp>,
        board: Entity<WorkGraphBoardView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.observe(&board, |_, _, cx| cx.notify());
        Self {
            app,
            board,
            _board_subscription: subscription,
        }
    }
}

impl Render for WorkGraphDetailView {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let app = self.app.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .px(theme().space.sm)
                    .py(theme().space.xs)
                    .border_b(theme().border)
                    .border_color(theme().colors.border)
                    .child(button(
                        "close-workgraph-inspector",
                        "Back to session details",
                        ButtonTone::Quiet,
                        true,
                        move |_, cx| {
                            let _ = app.update(cx, |app, cx| {
                                app.close_workgraph_inspector(cx);
                            });
                        },
                    )),
            )
            .child(
                div().flex_1().min_h_0().child(
                    self.board
                        .update(cx, |board, cx| board.render_external_detail(cx)),
                ),
            )
    }
}
