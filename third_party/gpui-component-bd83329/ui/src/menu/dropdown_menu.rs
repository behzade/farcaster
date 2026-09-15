use std::rc::Rc;

use gpui::{
    Anchor, Context, DismissEvent, ElementId, Entity, Focusable, InteractiveElement, IntoElement,
    MouseButton, RenderOnce, SharedString, StyleRefinement, Styled, Window,
    prelude::FluentBuilder as _,
};

use crate::{Selectable, button::Button, menu::PopupMenu, popover::Popover};

/// A dropdown menu trait for buttons and other interactive elements
pub trait DropdownMenu: Styled + Selectable + InteractiveElement + IntoElement + 'static {
    /// Create a dropdown menu with the given items, anchored to the TopLeft corner
    fn dropdown_menu(
        self,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> DropdownMenuPopover<Self> {
        self.dropdown_menu_with_anchor(Anchor::TopLeft, f)
    }

    /// Create a dropdown menu with the given items, anchored to the given corner
    fn dropdown_menu_with_anchor(
        mut self,
        anchor: impl Into<Anchor>,
        f: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> DropdownMenuPopover<Self> {
        let style = self.style().clone();
        let id = self.interactivity().element_id.clone();

        DropdownMenuPopover::new(id.unwrap_or(0.into()), anchor, self, f).trigger_style(style)
    }
}

impl DropdownMenu for Button {}

#[derive(IntoElement)]
pub struct DropdownMenuPopover<T: Selectable + IntoElement + 'static> {
    id: ElementId,
    style: StyleRefinement,
    anchor: Anchor,
    trigger: T,
    mouse_button: MouseButton,
    anchor_to_cursor: bool,
    builder: Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>,
    refresh_key: Option<SharedString>,
}

impl<T> DropdownMenuPopover<T>
where
    T: Selectable + IntoElement + 'static,
{
    fn new(
        id: ElementId,
        anchor: impl Into<Anchor>,
        trigger: T,
        builder: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        Self {
            id: SharedString::from(format!("dropdown-menu:{:?}", id)).into(),
            style: StyleRefinement::default(),
            anchor: anchor.into(),
            trigger,
            mouse_button: MouseButton::Left,
            anchor_to_cursor: false,
            builder: Rc::new(builder),
            refresh_key: None,
        }
    }

    /// Set the anchor corner for the dropdown menu popover.
    pub fn anchor(mut self, anchor: impl Into<Anchor>) -> Self {
        self.anchor = anchor.into();
        self
    }

    /// Set the mouse button that opens the menu.
    pub fn mouse_button(mut self, mouse_button: MouseButton) -> Self {
        self.mouse_button = mouse_button;
        self
    }

    /// Rebuild an open menu when the data used by its builder changes.
    pub fn refresh_key(mut self, key: impl Into<SharedString>) -> Self {
        self.refresh_key = Some(key.into());
        self
    }

    /// Place the configured menu anchor corner at the pointer position that opens it.
    /// Programmatic openings continue to use the trigger as their anchor.
    pub fn anchor_to_cursor(mut self) -> Self {
        self.anchor_to_cursor = true;
        self
    }

    /// Set the style refinement for the dropdown menu trigger.
    fn trigger_style(mut self, style: StyleRefinement) -> Self {
        self.style = style;
        self
    }
}

#[derive(Default)]
struct DropdownMenuState {
    menu: Option<Entity<PopupMenu>>,
    refresh_key: Option<SharedString>,
}

impl<T> RenderOnce for DropdownMenuPopover<T>
where
    T: Selectable + IntoElement + 'static,
{
    fn render(self, window: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let builder = self.builder.clone();
        let menu_state =
            window.use_keyed_state(self.id.clone(), cx, |_, _| DropdownMenuState::default());

        Popover::new(SharedString::from(format!("popover:{}", self.id)))
            .appearance(false)
            .overlay_closable(false)
            .mouse_button(self.mouse_button)
            .when(self.anchor_to_cursor, |popover| popover.anchor_to_cursor())
            .trigger(self.trigger)
            .trigger_style(self.style)
            .anchor(self.anchor)
            .content(move |_, window, cx| {
                menu_state.update(cx, |state, _| {
                    if state.refresh_key != self.refresh_key {
                        state.menu = None;
                        state.refresh_key = self.refresh_key.clone();
                    }
                });
                // Here is special logic to only create the PopupMenu once and reuse it.
                // Because this `content` will called in every time render, so we need to store the menu
                // in state to avoid recreating at every render.
                //
                // And we also need to rebuild the menu when it is dismissed, to rebuild menu items
                // dynamically for support `dropdown_menu` method, so we listen for DismissEvent below.
                let menu = match menu_state.read(cx).menu.clone() {
                    Some(menu) => menu,
                    None => {
                        let builder = builder.clone();
                        let menu = PopupMenu::build(window, cx, move |menu, window, cx| {
                            builder(menu, window, cx)
                        });
                        menu_state.update(cx, |state, _| {
                            state.menu = Some(menu.clone());
                        });
                        menu.focus_handle(cx).focus(window, cx);

                        // Listen for dismiss events from the PopupMenu to close the popover.
                        let popover_state = cx.entity();
                        window
                            .subscribe(&menu, cx, {
                                let menu_state = menu_state.clone();
                                move |_, _: &DismissEvent, window, cx| {
                                    popover_state.update(cx, |state, cx| {
                                        state.dismiss(window, cx);
                                    });
                                    menu_state.update(cx, |state, _| {
                                        state.menu = None;
                                    });
                                }
                            })
                            .detach();

                        menu.clone()
                    }
                };

                menu.clone()
            })
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use gpui::{
        AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled as _, Window,
        div, point, px,
    };

    use super::*;
    use crate::{button::Button, menu::PopupMenuItem, popover::Popover};

    struct HostHarness {
        selected: Rc<RefCell<bool>>,
    }

    impl Render for HostHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let selected = self.selected.clone();
            div().size_full().child(
                Popover::new("host-popover")
                    .trigger(
                        Button::new("host-trigger")
                            .label("Host")
                            .debug_selector(|| "host-trigger".into()),
                    )
                    .content(move |_, _, _| {
                        let selected = selected.clone();
                        div()
                            .debug_selector(|| "host-content".into())
                            .w(px(60.))
                            .h(px(40.))
                            .child(
                                Button::new("nested-trigger")
                                    .label("Nested")
                                    .debug_selector(|| "nested-trigger".into())
                                    .dropdown_menu(move |menu, _, _| {
                                        menu.item(PopupMenuItem::new("Item").on_click({
                                            let selected = selected.clone();
                                            move |_, _, _| *selected.borrow_mut() = true
                                        }))
                                    }),
                            )
                    }),
            )
        }
    }

    #[gpui::test]
    fn overhanging_nested_dropdown_press_keeps_the_host_popover_open(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(crate::init);
        let selected = Rc::new(RefCell::new(false));
        let (_, mut cx) = cx.add_window_view({
            let selected = selected.clone();
            move |_, _| HostHarness { selected }
        });

        cx.update(|window, cx| window.draw(cx).clear(cx));
        let host_trigger = cx
            .debug_bounds("host-trigger")
            .expect("host trigger rendered");
        cx.simulate_click(host_trigger.center(), Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let host_content = cx
            .debug_bounds("host-content")
            .expect("host popover content rendered");

        let nested_trigger = cx
            .debug_bounds("nested-trigger")
            .expect("nested trigger rendered");
        cx.simulate_click(nested_trigger.center(), Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let menu = cx.debug_bounds("popup-menu").expect("nested menu rendered");

        // Press the part of the menu item that overhangs the host popover's
        // right edge. The press is inside the nested menu, so the host must
        // not treat it as an outside click and dismiss itself.
        let press = point(menu.right() - px(10.), menu.top() + px(17.));
        assert!(press.x > host_content.right());
        cx.simulate_click(press, Default::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));

        assert!(
            *selected.borrow(),
            "the nested menu item click must be handled"
        );
        assert!(
            cx.debug_bounds("host-content").is_some(),
            "the host popover must stay open when a press lands in an overhanging nested menu"
        );
    }
}
