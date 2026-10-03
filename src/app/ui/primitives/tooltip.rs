use std::{
    cell::Cell,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{
    AnyElement, AnyView, App, Bounds, Context, Entity, EntityId, InteractiveElement, IntoElement,
    MouseButton, ParentElement as _, Pixels, Render, SharedString, Task, Window, deferred, div,
};
use gpui_component::{ElementExt, Placement, tooltip::Tooltip};

const SHOW_DELAY: Duration = Duration::from_millis(400);
const GRACE_PERIOD: Duration = Duration::from_millis(600);
type Builder = Rc<dyn Fn(&mut Window, &mut App) -> AnyView>;

#[derive(Default)]
pub(crate) struct AppTooltips {
    content: Option<(Bounds<Pixels>, Builder)>,
    owner: Option<EntityId>,
    last_hide: Option<Instant>,
    show_task: Option<Task<()>>,
    hide_task: Option<Task<()>>,
}

pub(crate) fn init_tooltips(cx: &mut App) {
    gpui_base::Root::register_plugin::<AppTooltips>(cx, |_, _| AppTooltips::default());
}

pub(crate) fn tooltip_overlay(window: &Window, cx: &App) -> Option<Entity<AppTooltips>> {
    window
        .root::<gpui_base::Root>()??
        .read(cx)
        .plugin::<AppTooltips>()
}

impl AppTooltips {
    pub(crate) fn is_visible(&self) -> bool {
        self.content.is_some()
    }

    fn show(
        &mut self,
        owner: EntityId,
        bounds: Bounds<Pixels>,
        build: Builder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.owner = Some(owner);
        self.hide_task = None;
        self.show_task = None;
        if self.content.is_some() || self.last_hide.is_some_and(|at| at.elapsed() < GRACE_PERIOD) {
            self.content = Some((bounds, build));
            cx.notify();
        } else {
            self.show_task = Some(cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(SHOW_DELAY).await;
                let _ = this.update_in(cx, |this, _, cx| {
                    this.content = Some((bounds, build));
                    cx.notify();
                });
            }));
        }
    }

    fn hide(&mut self, owner: EntityId, window: &mut Window, cx: &mut Context<Self>) {
        if self.owner != Some(owner) {
            return;
        }
        self.show_task = None;
        if self.content.is_none() {
            return;
        }
        self.last_hide = Some(Instant::now());
        self.hide_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(GRACE_PERIOD).await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.content = None;
                cx.notify();
            });
        }));
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.show_task = None;
        self.hide_task = None;
        self.last_hide = None;
        self.owner = None;
        if self.content.take().is_some() {
            cx.notify();
        }
    }
}

impl gpui_base::RootPlugin for AppTooltips {}
impl Render for AppTooltips {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some((bounds, build)) = &self.content else {
            return div().into_any_element();
        };
        deferred(
            gpui_base::TooltipPositioner::new(*bounds)
                .placement(Placement::Right)
                .child(build(window, cx)),
        )
        .with_priority(200)
        .into_any_element()
    }
}

pub(crate) fn tooltip(
    label: impl Into<SharedString> + 'static,
) -> impl Fn(&mut Window, &mut App) -> AnyView {
    let label = label.into();
    move |window, cx| Tooltip::new(label.clone()).build(window, cx)
}

pub(crate) trait AppTooltip: InteractiveElement + ElementExt + Sized {
    fn app_tooltip(self, label: impl Into<SharedString> + 'static) -> Self {
        self.app_tooltip_view(tooltip(label))
    }

    fn app_tooltip_element(
        self,
        content: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        let content = Rc::new(content);
        self.app_tooltip_view(move |window, cx| {
            let content = content.clone();
            Tooltip::element(move |window, cx| content(window, cx)).build(window, cx)
        })
    }

    fn app_tooltip_view(self, build: impl Fn(&mut Window, &mut App) -> AnyView + 'static) -> Self {
        let bounds = Rc::new(Cell::new((Bounds::default(), None)));
        let writer = bounds.clone();
        let build: Builder = Rc::new(build);
        let mut element = self.on_prepaint(move |bounds, window, cx| {
            let owner = window.use_keyed_state("app-tooltip-trigger", cx, |_, _| ());
            writer.set((bounds, Some(owner.entity_id())));
        });
        element
            .interactivity()
            .on_hover(move |hovered, window, cx| {
                let (bounds, owner) = bounds.get();
                if let Some(owner) = owner
                    && let Some(overlay) = tooltip_overlay(window, cx)
                {
                    overlay.update(cx, |overlay, cx| {
                        if *hovered {
                            overlay.show(owner, bounds, build.clone(), window, cx);
                        } else {
                            overlay.hide(owner, window, cx);
                        }
                    });
                }
            });
        element.on_mouse_down(MouseButton::Left, |_, window, cx| {
            if let Some(overlay) = tooltip_overlay(window, cx) {
                overlay.update(cx, |overlay, cx| overlay.clear(cx));
            }
        })
    }
}

impl<E: InteractiveElement + ElementExt> AppTooltip for E {}

#[cfg(test)]
#[path = "tooltip_tests.rs"]
mod tests;
