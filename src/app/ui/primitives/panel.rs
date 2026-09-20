use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, InteractiveElement as _, IntoElement, MouseDownEvent,
    ParentElement as _, RenderOnce, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div, prelude::FluentBuilder as _,
};

use gpui_component::scroll::ScrollableElement as _;

use crate::app::ui::{assets::AppIcon, theme::theme};

use super::{
    icon::{AppIconSize, app_icon},
    resize::{ResizeBounds, ResizeState, resize_handle},
    slot::number_slot,
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
    badge: Option<SharedString>,
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
            badge: None,
            body_inset: true,
            on_toggle: None,
            on_resize: None,
            children: Vec::new(),
        }
    }

    /// Content that carries its own padding — the archived chats, which are the
    /// same rows as the active list — sits flush against the panel so it lines
    /// up with the chats above it.
    pub(crate) fn flush_body(mut self) -> Self {
        self.body_inset = false;
        self
    }

    /// The unseen counter, written `+N` because it keeps climbing while you look
    /// away from the panel.
    pub(crate) fn badge(mut self, unseen: usize) -> Self {
        self.badge = (unseen > 0).then(|| format!("+{unseen}").into());
        self
    }

    /// A plain tally — the archived chats — which reads as a number, not a gain.
    pub(crate) fn count(mut self, total: usize) -> Self {
        self.badge = (total > 0).then(|| total.to_string().into());
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
            .pr(theme().size(10.0))
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
                    .id(ElementId::Name(format!("{id}-toggle").into()))
                    .w(theme().controls.icon_button)
                    .h(theme().controls.icon_button)
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme().colors.muted)
                    .hover(|arrow| arrow.bg(theme().colors.highlight))
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
            .when_some(badge, |header, badge| {
                header.child(number_slot(badge, theme().layout.counter_slot))
            });
        div()
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
            // Drawn last and over the top of the header: a minimized panel is
            // exactly its header, so it stands no taller than the highlight box
            // behind its toggle and never offers to resize, while an open one
            // keeps every pixel of its height for its rows.
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
