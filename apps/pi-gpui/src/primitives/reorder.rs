//! Reusable GPUI drag handles and before/after reorder targets.

use gpui::{
    AnyElement, App, AppContext as _, Bounds, Div, DragMoveEvent, ElementId,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Point, Render, Rgba, Role,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::tooltip::Tooltip;

use crate::{
    assets::AppIcon,
    primitives::{AppIconSize, app_icon},
    theme::THEME,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReorderPosition {
    Before,
    After,
}

pub(crate) fn reorder_handle<T>(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    drag: T,
    on_start: impl Fn(&mut App) + 'static,
) -> AnyElement
where
    T: Clone + Render + 'static,
{
    let label = label.into();
    let tooltip_label = label.clone();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .tab_index(0)
        .flex_none()
        .flex()
        .items_center()
        .mr(THEME.space.xs)
        .cursor(gpui::CursorStyle::OpenHand)
        .text_color(THEME.colors.subtle)
        .tooltip(move |window, cx| Tooltip::new(tooltip_label.clone()).build(window, cx))
        .on_drag(drag, move |drag, _, _, cx| {
            on_start(cx);
            cx.new(|_| drag.clone())
        })
        .child(app_icon(AppIcon::DotsSixVertical, AppIconSize::Control))
        .into_any_element()
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
mod tests {
    use gpui::{Bounds, point, size};

    use super::*;

    #[test]
    fn reorder_position_only_selects_the_row_under_the_pointer() {
        let row = Bounds::new(point(px(10.0), px(100.0)), size(px(200.0), px(40.0)));

        assert_eq!(
            reorder_position(&row, &point(px(20.0), px(110.0))),
            Some(ReorderPosition::Before)
        );
        assert_eq!(
            reorder_position(&row, &point(px(20.0), px(130.0))),
            Some(ReorderPosition::After)
        );
        assert_eq!(reorder_position(&row, &point(px(20.0), px(90.0))), None);
        assert_eq!(reorder_position(&row, &point(px(20.0), px(150.0))), None);
    }
}
