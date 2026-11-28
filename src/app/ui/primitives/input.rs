use std::{cell::Cell, rc::Rc};

use gpui::{
    AnyElement, App, Bounds, Element, ElementId, Entity, GlobalElementId, Hitbox, HitboxBehavior,
    InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseUpEvent, Pixels,
    RenderOnce, SharedString, StyleRefinement, Styled, Window,
};
use gpui_component::{
    Disableable,
    input::{Input as ComponentInput, InputState},
};

#[derive(IntoElement)]
pub(crate) struct Input {
    component: ComponentInput,
    state: Entity<InputState>,
    disabled: bool,
    appearance: bool,
}

impl Input {
    pub(crate) fn new(state: &Entity<InputState>) -> Self {
        Self {
            component: ComponentInput::new(state),
            state: state.clone(),
            disabled: false,
            appearance: true,
        }
    }

    pub(crate) fn appearance(mut self, appearance: bool) -> Self {
        self.component = self.component.appearance(appearance);
        self.appearance = appearance;
        self
    }

    pub(crate) fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.component = self.component.aria_label(label);
        self
    }
}

impl Disableable for Input {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self.component = self.component.disabled(disabled);
        self
    }
}

impl Styled for Input {
    fn style(&mut self) -> &mut StyleRefinement {
        self.component.style()
    }
}

impl RenderOnce for Input {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let component = if self.appearance && !self.disabled {
            self.component
                .bg(crate::app::ui::theme::theme().colors.canvas)
        } else {
            self.component
        };
        let pressed = window
            .use_keyed_state(("input-click-focus", self.state.entity_id()), cx, |_, _| {
                Rc::new(Cell::new(false))
            })
            .read(cx)
            .clone();
        InputClickFocus {
            child: component.into_any_element(),
            state: self.state,
            pressed,
            disabled: self.disabled,
        }
    }
}

struct InputClickFocus {
    child: AnyElement,
    state: Entity<InputState>,
    pressed: Rc<Cell<bool>>,
    disabled: bool,
}

impl IntoElement for InputClickFocus {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for InputClickFocus {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

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
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        self.child.prepaint(window, cx);
        hitbox
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        let down_hitbox = hitbox.clone();
        let pressed = self.pressed.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, _| {
            if phase == gpui::DispatchPhase::Capture && event.button == MouseButton::Left {
                pressed.set(down_hitbox.is_hovered(window));
            }
        });
        let up_hitbox = hitbox.clone();
        let pressed = self.pressed.clone();
        let state = self.state.clone();
        let disabled = self.disabled;
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase == gpui::DispatchPhase::Bubble
                && event.button == MouseButton::Left
                && pressed.replace(false)
                && up_hitbox.is_hovered(window)
                && !disabled
            {
                state.update(cx, |state, cx| {
                    state.focus(window, cx);
                    cx.notify();
                });
                cx.stop_propagation();
            }
        });
        self.child.paint(window, cx);
    }
}

#[cfg(test)]
#[path = "input_tests.rs"]
mod tests;
