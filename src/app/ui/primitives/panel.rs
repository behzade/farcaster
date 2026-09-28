use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, FontWeight, InteractiveElement as _, IntoElement, MouseDownEvent,
    ParentElement as _, RenderOnce, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, prelude::FluentBuilder as _,
};

use gpui_component::scroll::ScrollableElement as _;

use crate::app::ui::theme::theme;

use super::{
    resize::{ResizeBounds, ResizeState, resize_handle},
    tooltip::AppTooltip as _,
};

type PanelToggle = Rc<dyn Fn(&mut Window, &mut App)>;
type PanelResize = Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub(crate) struct Panel {
    id: SharedString,
    state: ResizeState,
    bounds: ResizeBounds,
    title: SharedString,
    count: usize,
    body_inset: bool,
    on_toggle: Option<PanelToggle>,
    on_resize: Option<PanelResize>,
    children: Vec<AnyElement>,
}

impl Panel {
    pub(crate) fn new(
        id: impl Into<SharedString>,
        state: &ResizeState,
        bounds: ResizeBounds,
        title: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            state: *state,
            bounds,
            title: title.into(),
            count: 0,
            body_inset: true,
            on_toggle: None,
            on_resize: None,
            children: Vec::new(),
        }
    }

    pub(crate) fn flush_body(mut self) -> Self {
        self.body_inset = false;
        self
    }

    pub(crate) fn count(mut self, total: usize) -> Self {
        self.count = total;
        self
    }

    pub(crate) fn on_toggle(self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self {
            on_toggle: Some(Rc::new(handler)),
            ..self
        }
    }

    pub(crate) fn on_resize(
        self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            on_resize: Some(Rc::new(handler)),
            ..self
        }
    }

    pub(crate) fn children(mut self, elements: impl IntoIterator<Item = AnyElement>) -> Self {
        self.children.extend(elements);
        self
    }
}

impl RenderOnce for Panel {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Self {
            id,
            state,
            bounds,
            title,
            count,
            body_inset,
            on_toggle,
            on_resize,
            children,
        } = self;
        let collapsed = state.is_collapsed();
        let height = state.height(bounds);
        let toggle = if collapsed {
            format!("Show {title}")
        } else {
            format!("Hide {title}")
        };
        let header = div()
            .id(ElementId::Name(id.clone()))
            .h(theme().controls.icon_button)
            .flex_none()
            .flex()
            .items_center()
            .gap(theme().space.xs)
            .px(theme().space.sm)
            .text_size(theme().type_scale.caption)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme().colors.muted)
            .cursor_pointer()
            .aria_label(toggle.clone())
            .app_tooltip(toggle)
            .hover(|header| header.bg(theme().colors.highlight))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                if let Some(on_toggle) = on_toggle.as_ref() {
                    on_toggle(window, cx);
                }
            })
            .child(
                div()
                    .min_w_0()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(title),
            )
            .when(collapsed, |header| {
                header.child(
                    div()
                        .flex_none()
                        .text_color(theme().colors.subtle)
                        .child(format!("({count})")),
                )
            });
        div()
            .id(ElementId::Name(format!("{id}-panel").into()))
            .relative()
            .flex_none()
            .flex()
            .flex_col()
            .when(!collapsed, |panel| panel.h(height))
            .child(header)
            .when(!collapsed, |panel| {
                panel.child(
                    div()
                        .id(ElementId::Name(format!("{id}-body").into()))
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .gap(theme().space.xs)
                        .when(body_inset, |body| {
                            body.px(theme().size(10.0)).pb(theme().space.sm)
                        })
                        .children(children)
                        .overflow_y_scrollbar(),
                )
            })
            .when(!collapsed, |panel| {
                panel.child(resize_handle(
                    ElementId::Name(format!("{id}-resize").into()),
                    move |event, window, cx| {
                        if let Some(on_resize) = on_resize.as_ref() {
                            on_resize(event, window, cx);
                        }
                    },
                ))
            })
    }
}
#[cfg(test)]
#[path = "panel_tests.rs"]
mod tests;
