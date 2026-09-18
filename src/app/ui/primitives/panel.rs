use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent,
    ParentElement as _, Pixels, RenderOnce, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _,
};

use gpui_component::scroll::ScrollableElement as _;

use crate::app::ui::{assets::AppIcon, theme::theme};

use super::{
    icon::{AppIconSize, app_icon},
    tooltip::AppTooltip as _,
};

#[derive(Clone, Copy)]
pub(crate) struct PanelBounds {
    pub height: Pixels,
    pub min_height: Pixels,
    pub max_height: Pixels,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct PanelState {
    height: Option<Pixels>,
    collapsed: bool,
    resize_start: Option<(Pixels, Pixels)>,
}

impl PanelState {
    pub(crate) fn height(&self, bounds: PanelBounds) -> Pixels {
        clamp(self.height.unwrap_or(bounds.height), bounds)
    }

    pub(crate) fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    pub(crate) fn set_collapsed(&mut self, collapsed: bool) {
        self.collapsed = collapsed;
    }

    pub(crate) fn begin_resize(&mut self, bounds: PanelBounds, pointer_y: Pixels) {
        self.resize_start = Some((pointer_y, self.height(bounds)));
    }

    pub(crate) fn update_resize(&mut self, bounds: PanelBounds, pointer_y: Pixels) -> bool {
        let Some((start_y, start_height)) = self.resize_start else {
            return false;
        };
        let height = clamp(start_height + start_y - pointer_y, bounds);
        if Some(height) == self.height.map(|height| clamp(height, bounds)) {
            return false;
        }
        self.height = Some(height);
        true
    }

    pub(crate) fn finish_resize(&mut self) -> bool {
        self.resize_start.take().is_some()
    }
}

fn clamp(height: Pixels, bounds: PanelBounds) -> Pixels {
    height.clamp(bounds.min_height, bounds.max_height)
}

#[derive(IntoElement)]
pub(crate) struct Panel {
    id: SharedString,
    state: PanelState,
    bounds: PanelBounds,
    title: SharedString,
    badge: Option<usize>,
    on_toggle: Option<Rc<dyn Fn(&mut Window, &mut App)>>,
    on_resize: Option<Rc<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>>,
    children: Vec<AnyElement>,
}

impl Panel {
    pub(crate) fn new(
        id: impl Into<SharedString>,
        state: &PanelState,
        bounds: PanelBounds,
        title: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            state: *state,
            bounds,
            title: title.into(),
            badge: None,
            on_toggle: None,
            on_resize: None,
            children: Vec::new(),
        }
    }

    pub(crate) fn badge(mut self, unseen: usize) -> Self {
        self.badge = (unseen > 0).then_some(unseen);
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
            badge,
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
            .px(theme().size(10.0))
            .border_t(theme().border)
            .border_color(theme().colors.border)
            .cursor_pointer()
            .aria_label(toggle.clone())
            .app_tooltip(toggle)
            .when(!collapsed, |header| header.bg(theme().colors.panel))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                if let Some(on_toggle) = on_toggle.as_ref() {
                    on_toggle(window, cx);
                }
            })
            .child(
                div()
                    .flex_none()
                    .text_color(theme().colors.muted)
                    .child(app_icon(
                        if collapsed {
                            AppIcon::CaretRight
                        } else {
                            AppIcon::CaretDown
                        },
                        AppIconSize::Inline,
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(theme().type_scale.caption)
                    .text_color(theme().colors.muted)
                    .child(title),
            )
            .when_some(badge, |header, unseen| {
                header.child(
                    div()
                        .flex_none()
                        .px(theme().space.xs)
                        .bg(theme().colors.highlight)
                        .text_size(theme().type_scale.caption)
                        .text_color(theme().colors.text)
                        .child(format!("+{unseen}")),
                )
            });
        div()
            .flex_none()
            .flex()
            .flex_col()
            .when(!collapsed, |panel| panel.h(height))
            .child(
                div()
                    .id(ElementId::Name(format!("{id}-resize").into()))
                    .h(theme().size(5.0))
                    .w_full()
                    .flex_none()
                    .cursor_row_resize()
                    .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                        cx.stop_propagation();
                        if let Some(on_resize) = on_resize.as_ref() {
                            on_resize(event, window, cx);
                        }
                    }),
            )
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
                        .px(theme().size(10.0))
                        .pb(theme().space.sm)
                        .children(children)
                        .overflow_y_scrollbar(),
                )
            })
    }
}

#[cfg(test)]
#[path = "panel_tests.rs"]
mod tests;
