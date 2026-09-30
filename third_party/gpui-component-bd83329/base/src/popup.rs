use std::{cell::Cell, rc::Rc};

use gpui::{
    Anchor, AnyElement, App, Bounds, Div, Element, ElementId, Entity, Global, GlobalElementId,
    Hitbox, HitboxBehavior, InspectorElementId, InteractiveElement, Interactivity, IntoElement,
    LayoutId, ParentElement, Pixels, Point, RenderOnce, StatefulInteractiveElement,
    StyleRefinement, Styled, WeakEntity, Window, WindowId, deferred, div, px,
};

use crate::{ElementExt as _, Positioner, StyledExt as _};

/// Distance kept between a popup and the window edge.
const WINDOW_MARGIN: Pixels = px(8.);

/// Deferred paint priority for interactive surfaces that must appear above dialogs.
pub const POPUP_PRIORITY: usize = 100;

#[derive(Default)]
struct PopupAnchorState {
    bounds: Bounds<Pixels>,
    captured: bool,
    surface: Option<Hitbox>,
}

#[derive(Default)]
struct PopupSurfaces(Vec<(WindowId, WeakEntity<PopupAnchorState>)>);

impl Global for PopupSurfaces {}

pub(crate) fn mouse_press_in_popup(window: &Window, cx: &App) -> bool {
    cx.try_global::<PopupSurfaces>().is_some_and(|surfaces| {
        let window_id = window.window_handle().window_id();
        surfaces.0.iter().any(|(id, state)| {
            *id == window_id
                && state.upgrade().is_some_and(|state| {
                    state
                        .read(cx)
                        .surface
                        .as_ref()
                        .is_some_and(|hitbox| hitbox.is_hovered(window))
                })
        })
    })
}

struct PopupSurface {
    content: AnyElement,
    state: Entity<PopupAnchorState>,
}

impl IntoElement for PopupSurface {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for PopupSurface {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.content.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.content.prepaint(window, cx);
        // Insert after the content's occluding hitbox so this marker remains
        // visible to hit testing without blocking any content interactions.
        let surface = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        self.state
            .update(cx, |state, _| state.surface = Some(surface));
        if !cx.has_global::<PopupSurfaces>() {
            cx.set_global(PopupSurfaces::default());
        }
        let surfaces = &mut cx.global_mut::<PopupSurfaces>().0;
        surfaces.retain(|(_, state)| state.upgrade().is_some());
        if !surfaces
            .iter()
            .any(|(_, state)| state.entity_id() == self.state.entity_id())
        {
            surfaces.push((window.window_handle().window_id(), self.state.downgrade()));
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.content.paint(window, cx);
    }
}

/// An unstyled trigger and anchored popup host.
///
/// `Popup` owns trigger measurement, anchor-point calculation, first-frame
/// synchronization, deferred rendering, and window-edge snapping. Callers own
/// open state, interaction, popup content, appearance, and motion.
#[derive(IntoElement)]
pub struct Popup {
    id: ElementId,
    base: gpui::Stateful<Div>,
    style: StyleRefinement,
    anchor: Anchor,
    anchor_position: Option<Point<Pixels>>,
    trigger: AnyElement,
    content: Option<AnyElement>,
}

impl Popup {
    pub fn new(id: impl Into<ElementId>, trigger: impl IntoElement) -> Self {
        let id = id.into();
        Self {
            base: div().id(id.clone()),
            id,
            style: StyleRefinement::default(),
            anchor: Anchor::TopLeft,
            anchor_position: None,
            trigger: trigger.into_any_element(),
            content: None,
        }
    }

    pub fn anchor(mut self, anchor: impl Into<Anchor>) -> Self {
        self.anchor = anchor.into();
        self
    }

    /// Position the popup at a window-relative point instead of at the trigger.
    pub fn position(mut self, position: Point<Pixels>) -> Self {
        self.anchor_position = Some(position);
        self
    }

    pub fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self
    }

    pub fn resolved_corner(anchor: Anchor, trigger_bounds: Bounds<Pixels>) -> Point<Pixels> {
        match anchor {
            Anchor::TopLeft => trigger_bounds.origin,
            Anchor::TopCenter => trigger_bounds.top_center(),
            Anchor::TopRight => trigger_bounds.top_right(),
            Anchor::BottomLeft => Point {
                x: trigger_bounds.origin.x,
                y: trigger_bounds.origin.y - trigger_bounds.size.height,
            },
            Anchor::BottomCenter => Point {
                x: trigger_bounds.top_center().x,
                y: trigger_bounds.origin.y - trigger_bounds.size.height,
            },
            Anchor::BottomRight => Point {
                x: trigger_bounds.top_right().x,
                y: trigger_bounds.origin.y - trigger_bounds.size.height,
            },
            Anchor::LeftCenter | Anchor::RightCenter => trigger_bounds.origin,
        }
    }
}

impl Styled for Popup {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl InteractiveElement for Popup {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for Popup {}

impl RenderOnce for Popup {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state =
            window.use_keyed_state((self.id, "anchor"), cx, |_, _| PopupAnchorState::default());
        let anchor = self.anchor;
        let anchor_position = self.anchor_position;
        let position =
            Rc::new(Cell::new(anchor_position.unwrap_or_else(|| {
                Self::resolved_corner(anchor, state.read(cx).bounds)
            })));

        let root = self
            .base
            .child(self.trigger)
            .on_prepaint({
                let state = state.clone();
                let position = position.clone();
                move |bounds, window, cx| {
                    if anchor_position.is_none() {
                        position.set(Self::resolved_corner(anchor, bounds));
                    }
                    let first = state.update(cx, |state, _| {
                        let first = !state.captured;
                        state.bounds = bounds;
                        state.captured = true;
                        first
                    });
                    if first {
                        window.request_animation_frame();
                    }
                }
            })
            .refine_style(&self.style);

        let Some(content) = self.content else {
            return root;
        };
        if !state.read(cx).captured {
            return root;
        }

        root.child(
            deferred(
                Positioner::corner(anchor, position.get())
                    .margin(WINDOW_MARGIN)
                    .child(PopupSurface { content, state }),
            )
            .with_priority(POPUP_PRIORITY),
        )
    }
}

#[cfg(test)]
#[path = "popup_surface_tests.rs"]
mod surface_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, Render};

    #[test]
    fn resolved_corner_preserves_existing_anchor_math() {
        let bounds = Bounds {
            origin: Point::new(px(100.), px(100.)),
            size: gpui::Size::new(px(200.), px(50.)),
        };
        assert_eq!(
            Popup::resolved_corner(Anchor::TopCenter, bounds),
            Point::new(px(200.), px(100.))
        );
        assert_eq!(
            Popup::resolved_corner(Anchor::BottomRight, bounds),
            Point::new(px(300.), px(50.))
        );
    }

    struct Harness;

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            Popup::new(
                "popup",
                div()
                    .debug_selector(|| "popup-trigger".into())
                    .size(px(100.)),
            )
            .content(
                div()
                    .debug_selector(|| "popup-content".into())
                    .size(px(20.)),
            )
        }
    }

    #[gpui::test]
    fn trigger_capture_enables_deferred_content_on_the_next_frame(cx: &mut gpui::TestAppContext) {
        let (_, window) = cx.add_window_view(|_, _| Harness);
        window.update(|window, cx| window.draw(cx).clear(cx));
        window.update(|window, cx| window.draw(cx).clear(cx));

        assert_eq!(
            window.debug_bounds("popup-trigger").unwrap().size,
            gpui::Size::new(px(100.), px(100.))
        );
        assert_eq!(
            window.debug_bounds("popup-content").unwrap().size,
            gpui::Size::new(px(20.), px(20.))
        );
    }
}
