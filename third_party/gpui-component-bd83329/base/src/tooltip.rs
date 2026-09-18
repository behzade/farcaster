use std::{
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{
    AnyElement, AnyView, App, Bounds, Context, Div, ElementId, InteractiveElement, IntoElement,
    ParentElement, Pixels, Render, RenderOnce, Role, Stateful, StatefulInteractiveElement, Styled,
    Task, Window, deferred, div, prelude::FluentBuilder as _, px,
};

use crate::{Placement, Positioner};

const WINDOW_MARGIN: Pixels = px(4.);
// Long enough that moving the pointer from one trigger to a sibling one keeps
// the tooltip on screen and swaps its content instead of hiding and waiting out
// the show delay again.
const GRACE_PERIOD: Duration = Duration::from_millis(600);
const SHOW_DELAY: Duration = Duration::from_millis(400);

type TooltipBuilder = Rc<dyn Fn(&mut Window, &mut App) -> AnyView>;
type TooltipRenderer = Rc<dyn Fn(AnyView, TooltipTransition, &mut Window, &mut App) -> AnyElement>;

/// An unstyled tooltip popup.
///
/// This corresponds to Base UI's `Tooltip.Popup`: it owns the accessible
/// tooltip role and accepts application-owned content and presentation.
#[derive(IntoElement)]
pub struct Tooltip {
    base: Stateful<Div>,
}

impl Tooltip {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id).role(Role::Tooltip),
        }
    }
}

impl Styled for Tooltip {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        self.base.style()
    }
}

impl ParentElement for Tooltip {
    fn extend(&mut self, children: impl IntoIterator<Item = AnyElement>) {
        self.base.extend(children);
    }
}

impl RenderOnce for Tooltip {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.base
    }
}

/// Content requested by a tooltip trigger.
#[derive(Clone)]
pub struct TooltipRequest {
    build: TooltipBuilder,
    trigger_bounds: Bounds<Pixels>,
    preferred_placement: Option<Placement>,
}

impl TooltipRequest {
    pub fn new(
        trigger_bounds: Bounds<Pixels>,
        build: impl Fn(&mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        Self {
            build: Rc::new(build),
            trigger_bounds,
            preferred_placement: None,
        }
    }

    pub fn placement(mut self, placement: Placement) -> Self {
        self.preferred_placement = Some(placement);
        self
    }
}

/// Presentation transition requested by the Base tooltip lifecycle.
#[derive(Clone, Copy, Debug)]
pub enum TooltipTransition {
    Enter {
        epoch: usize,
    },
    Switch {
        epoch: usize,
        previous: Bounds<Pixels>,
        current: Bounds<Pixels>,
    },
}

/// Per-window tooltip provider and overlay.
pub struct TooltipOverlay {
    content: Option<TooltipRequest>,
    previous_bounds: Option<Bounds<Pixels>>,
    epoch: usize,
    last_hide: Option<Instant>,
    animation_epoch: usize,
    is_switching: bool,
    show_task: Option<Task<()>>,
    hide_task: Option<Task<()>>,
    renderer: TooltipRenderer,
}

impl TooltipOverlay {
    pub fn new() -> Self {
        Self {
            content: None,
            previous_bounds: None,
            epoch: 0,
            last_hide: None,
            animation_epoch: 0,
            is_switching: false,
            show_task: None,
            hide_task: None,
            renderer: Rc::new(|view, _, _, _| div().child(view).into_any_element()),
        }
    }

    pub fn is_visible(&self) -> bool {
        self.content.is_some()
    }

    /// Whether a tooltip was hidden recently enough that the next one may
    /// appear without the show delay. Reading the deadline instead of latching
    /// a flag keeps the delay honest: cancelling the hide timer (which happens
    /// whenever the pointer lands on a sibling trigger) can no longer leave
    /// every later tooltip immediate.
    fn had_recent_tooltip(&self) -> bool {
        self.last_hide
            .is_some_and(|hidden_at| hidden_at.elapsed() < GRACE_PERIOD)
    }

    pub fn render_with(
        mut self,
        renderer: impl Fn(AnyView, TooltipTransition, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        self.renderer = Rc::new(renderer);
        self
    }

    fn next_epoch(&mut self) -> usize {
        self.epoch += 1;
        self.epoch
    }

    pub fn request_show(
        &mut self,
        content: TooltipRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hide_task = None;
        let was_visible = self.content.is_some();
        if was_visible || self.had_recent_tooltip() {
            self.previous_bounds = self.content.as_ref().map(|content| content.trigger_bounds);
            self.content = Some(content);
            self.show_task = None;
            self.is_switching = was_visible;
            self.animation_epoch += 1;
            cx.notify();
            return;
        }

        let epoch = self.next_epoch();
        self.show_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(SHOW_DELAY).await;
            let _ = this.update_in(cx, |this, _, cx| {
                if this.epoch == epoch {
                    this.content = Some(content);
                    this.previous_bounds = None;
                    this.is_switching = false;
                    this.animation_epoch += 1;
                    cx.notify();
                }
            });
        }));
    }

    pub fn request_hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_task = None;
        if self.content.is_none() {
            return;
        }
        let epoch = self.next_epoch();
        self.last_hide = Some(Instant::now());
        self.hide_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(GRACE_PERIOD).await;
            let _ = this.update_in(cx, |this, _, cx| {
                if this.epoch == epoch {
                    this.content = None;
                    this.previous_bounds = None;
                    cx.notify();
                }
            });
        }));
    }

    pub fn hide(&mut self, cx: &mut Context<Self>) {
        let changed = self.content.is_some()
            || self.previous_bounds.is_some()
            || self.had_recent_tooltip()
            || self.show_task.is_some()
            || self.hide_task.is_some();
        self.content = None;
        self.previous_bounds = None;
        self.last_hide = None;
        self.is_switching = false;
        self.show_task = None;
        self.hide_task = None;
        if changed {
            cx.notify();
        }
    }
}

impl Default for TooltipOverlay {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for TooltipOverlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(content) = self.content.as_ref() else {
            return div().into_any_element();
        };
        let view = (content.build)(window, cx);
        let transition = match (self.is_switching, self.previous_bounds) {
            (true, Some(previous)) => TooltipTransition::Switch {
                epoch: self.animation_epoch,
                previous,
                current: content.trigger_bounds,
            },
            _ => TooltipTransition::Enter {
                epoch: self.animation_epoch,
            },
        };
        let rendered = (self.renderer)(view, transition, window, cx);
        deferred(
            TooltipPositioner::new(content.trigger_bounds)
                .when_some(content.preferred_placement, |this, placement| {
                    this.placement(placement)
                })
                .child(rendered),
        )
        .with_priority(2)
        .into_any_element()
    }
}

/// An unstyled tooltip positioner with viewport-aware flipping and clamping.
///
/// This is a tooltip-named view of [`crate::Positioner`]'s side placement. It
/// adds no element of its own; the shared positioner is what gets rendered.
pub struct TooltipPositioner(Positioner);

impl TooltipPositioner {
    pub fn new(trigger_bounds: Bounds<Pixels>) -> Self {
        Self(Positioner::side(trigger_bounds).margin(WINDOW_MARGIN))
    }

    pub fn placement(mut self, placement: Placement) -> Self {
        self.0 = self.0.placement(placement);
        self
    }
}

impl ParentElement for TooltipPositioner {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.0.extend(elements);
    }
}

impl IntoElement for TooltipPositioner {
    type Element = Positioner;

    fn into_element(self) -> Self::Element {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, point, size};

    fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
    }

    #[gpui::test]
    fn provider_owns_grace_switch_and_dismiss(cx: &mut gpui::TestAppContext) {
        let state = cx.update(|cx| cx.new(|_| TooltipOverlay::new()));
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            state.update(cx, |tooltip, cx| {
                tooltip.last_hide = Some(Instant::now());
                tooltip.request_show(
                    TooltipRequest::new(bounds(0., 0., 20., 20.), |_, _| {
                        panic!("content is not rendered by this lifecycle test")
                    }),
                    window,
                    cx,
                );
            });
        });
        cx.update(|_, cx| assert!(state.read(cx).content.is_some()));

        cx.update(|_, cx| {
            state.update(cx, |tooltip, cx| tooltip.hide(cx));
        });
        cx.update(|_, cx| assert!(state.read(cx).content.is_none()));
    }

    #[gpui::test]
    fn grace_expires_so_the_next_hover_pays_the_show_delay_again(
        cx: &mut gpui::TestAppContext,
    ) {
        let state = cx.update(|cx| cx.new(|_| TooltipOverlay::new()));
        let cx = cx.add_empty_window();
        cx.update(|window, cx| {
            state.update(cx, |tooltip, cx| {
                tooltip.content = Some(TooltipRequest::new(bounds(0., 0., 20., 20.), |_, _| {
                    panic!("content is not rendered by this lifecycle test")
                }));
                tooltip.request_hide(window, cx);
                assert!(tooltip.had_recent_tooltip());
            });
        });

        cx.update(|_, cx| {
            state.update(cx, |tooltip, _| {
                tooltip.last_hide = Some(Instant::now() - GRACE_PERIOD * 2);
            });
        });
        cx.update(|_, cx| assert!(!state.read(cx).had_recent_tooltip()));
    }
}
