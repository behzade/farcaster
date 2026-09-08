use gpui::{
    App, Bounds, Div, DragMoveEvent, InteractiveElement as _, Pixels, Point, Rgba, Stateful,
    Styled as _, Window, prelude::FluentBuilder as _, px,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReorderPosition {
    Before,
    After,
}

pub(crate) trait ReorderTargetExt {
    fn reorder_target<T>(
        self,
        position: Option<ReorderPosition>,
        indicator: Rgba,
        hover: Rgba,
        on_move: impl Fn(ReorderPosition, &mut Window, &mut App) + 'static,
        on_drop: impl Fn(&T, &mut Window, &mut App) + 'static,
    ) -> Self
    where
        T: 'static;
}

impl ReorderTargetExt for Stateful<Div> {
    fn reorder_target<T>(
        self,
        position: Option<ReorderPosition>,
        indicator: Rgba,
        hover: Rgba,
        on_move: impl Fn(ReorderPosition, &mut Window, &mut App) + 'static,
        on_drop: impl Fn(&T, &mut Window, &mut App) + 'static,
    ) -> Self
    where
        T: 'static,
    {
        reorder_target(self, position, indicator, hover, on_move, on_drop)
    }
}

fn reorder_position(bounds: &Bounds<Pixels>, pointer: &Point<Pixels>) -> Option<ReorderPosition> {
    bounds.contains(pointer).then(|| {
        if pointer.y < bounds.center().y {
            ReorderPosition::Before
        } else {
            ReorderPosition::After
        }
    })
}

fn reorder_target<T>(
    row: Stateful<Div>,
    position: Option<ReorderPosition>,
    indicator: Rgba,
    hover: Rgba,
    on_move: impl Fn(ReorderPosition, &mut Window, &mut App) + 'static,
    on_drop: impl Fn(&T, &mut Window, &mut App) + 'static,
) -> Stateful<Div>
where
    T: 'static,
{
    row.when(position == Some(ReorderPosition::Before), |row| {
        row.border_t(px(2.0)).border_color(indicator)
    })
    .when(position == Some(ReorderPosition::After), |row| {
        row.border_b(px(2.0)).border_color(indicator)
    })
    .on_drag_move(move |event: &DragMoveEvent<T>, window, cx| {
        if let Some(position) = reorder_position(&event.bounds, &event.event.position) {
            on_move(position, window, cx);
        }
    })
    .drag_over::<T>(move |row, _, _, _| row.bg(hover))
    .on_drop(on_drop)
}

#[cfg(test)]
#[path = "reorder_tests.rs"]
mod tests;
