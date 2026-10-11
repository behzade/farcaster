use std::rc::Rc;

use gpui::{
    Anchor, AnyElement, App, Bounds, Context, ElementId, Focusable as _, InteractiveElement as _,
    IntoElement, MouseButton, ParentElement as _, Pixels, RenderOnce, SharedString,
    StyleRefinement, Styled, Window, div, point, px,
};
use gpui_base::{ElementExt as _, PopoverState};
use gpui_component::{Selectable, ThemeStyled as _};

use crate::app::views::native_overlay::{native::OverlayInput, popup};

type Content = Rc<dyn Fn(&mut PopoverState, &mut Window, &mut Context<PopoverState>) -> AnyElement>;
type OpenChange = Rc<dyn Fn(&bool, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub(crate) struct Popover {
    id: ElementId,
    anchor: Anchor,
    appearance: bool,
    open: Option<bool>,
    on_open_change: Option<OpenChange>,
    trigger: Option<Box<dyn FnOnce(bool) -> AnyElement>>,
    content: Option<Content>,
    mouse_button: MouseButton,
    at_pointer: bool,
}

impl Popover {
    pub(crate) fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            anchor: Anchor::TopLeft,
            appearance: true,
            open: None,
            on_open_change: None,
            trigger: None,
            content: None,
            mouse_button: MouseButton::Left,
            at_pointer: false,
        }
    }
    pub(crate) fn anchor(mut self, anchor: Anchor) -> Self {
        self.anchor = anchor;
        self
    }
    pub(crate) fn appearance(mut self, appearance: bool) -> Self {
        self.appearance = appearance;
        self
    }
    pub(crate) fn open(mut self, open: bool) -> Self {
        self.open = Some(open);
        self
    }
    pub(crate) fn trigger<T: Selectable + IntoElement + 'static>(mut self, trigger: T) -> Self {
        self.trigger = Some(Box::new(move |open| trigger.open(open).into_any_element()));
        self
    }
    pub(crate) fn on_open_change(
        mut self,
        callback: impl Fn(&bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_open_change = Some(Rc::new(callback));
        self
    }
    pub(crate) fn content<E: IntoElement>(
        mut self,
        content: impl Fn(&mut PopoverState, &mut Window, &mut Context<PopoverState>) -> E + 'static,
    ) -> Self {
        self.content = Some(Rc::new(move |state, window, cx| {
            content(state, window, cx).into_any_element()
        }));
        self
    }
}

impl RenderOnce for Popover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(trigger) = self.trigger else {
            return div().into_any_element();
        };
        let panel = window.use_keyed_state((self.id.clone(), "panel"), cx, |_, _| {
            popup::PopupState::default()
        });
        // Ordinary GPUI windows, including the modal panel, already draw above the editor.
        if !popup::uses_native_popups(window, cx) || panel.read(cx).failed {
            popup::prepare(
                panel,
                false,
                Rc::new(|_, _| div().into_any_element()),
                OverlayInput::Modal,
                None,
                window,
                cx,
            );
            return gpui_component::popover::Popover::new(self.id)
                .anchor(self.anchor)
                .appearance(self.appearance)
                .mouse_button(self.mouse_button)
                .trigger(Trigger {
                    build: trigger,
                    open: false,
                })
                .when_some(self.open, |popover, open| popover.open(open))
                .when_some(self.on_open_change, |popover, callback| {
                    popover.on_open_change(move |open, window, cx| callback(open, window, cx))
                })
                .when_some(self.content, |popover, content| {
                    popover.content(move |state, window, cx| content(state, window, cx))
                })
                .into_any_element();
        }
        let state =
            window.use_keyed_state(self.id.clone(), cx, |_, cx| PopoverState::new(false, cx));
        state.update(cx, |state, cx| {
            state.set_on_open_change(self.on_open_change);
            if let Some(open) = self.open {
                state.sync_open(open, window, cx);
            }
        });
        let geometry = window.use_keyed_state((self.id.clone(), "geometry"), cx, |_, _| {
            (Bounds::<Pixels>::default(), None)
        });
        let parent = window.current_view();
        let owner_window = window.window_handle();
        let open = state.read(cx).is_open();
        let focus = state.read(cx).focus_handle(cx);
        let anchor = self.anchor;
        let appearance = self.appearance;
        let at_pointer = self.at_pointer;
        let content = self.content;
        let build_state = state.clone();
        let dismiss_state = state.clone();
        let dismiss: popup::Dismiss = Rc::new(move |window, cx| {
            let state = dismiss_state.clone();
            owner_window
                .update(cx, |_, owner, cx| {
                    state.update(cx, |state, cx| state.dismiss(owner, cx));
                    owner.refresh();
                })
                .ok();
            let _ = window;
        });
        let backdrop_dismiss = dismiss.clone();
        popup::prepare(
            panel,
            open,
            Rc::new(move |window, cx| {
                let (bounds, pointer) = *geometry.read(cx);
                let position = if at_pointer {
                    pointer.unwrap_or(bounds.origin)
                } else {
                    // Match the component's default quarter-rem gap at the trigger edge.
                    let gap = window.rem_size() * 0.25;
                    match anchor {
                        Anchor::TopLeft => bounds.bottom_left() + point(px(0.0), gap),
                        Anchor::TopCenter => bounds.bottom_center() + point(px(0.0), gap),
                        Anchor::TopRight => bounds.bottom_right() + point(px(0.0), gap),
                        Anchor::BottomLeft => bounds.origin - point(px(0.0), gap),
                        Anchor::BottomCenter => bounds.top_center() - point(px(0.0), gap),
                        Anchor::BottomRight => bounds.top_right() - point(px(0.0), gap),
                        Anchor::LeftCenter => bounds.right_center() + point(gap, px(0.0)),
                        Anchor::RightCenter => bounds.left_center() - point(gap, px(0.0)),
                    }
                };
                let surface = div()
                    .id("native-popover-content")
                    .occlude()
                    .tab_group()
                    .track_focus(&focus)
                    .key_context("Popover")
                    .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
                    .when(appearance, |surface| surface.popover_style(cx).p_3())
                    .children(content.as_ref().map(|content| {
                        build_state.update(cx, |state, cx| content(state, window, cx))
                    }));
                div()
                    .size_full()
                    .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                    .on_mouse_down(gpui::MouseButton::Left, {
                        let dismiss = backdrop_dismiss.clone();
                        move |_, window, cx| {
                            dismiss(window, cx);
                            cx.stop_propagation();
                        }
                    })
                    .child(
                        gpui_base::Positioner::corner(anchor, position)
                            .margin(px(8.0))
                            .occlude()
                            .child(surface),
                    )
                    .into_any_element()
            }),
            OverlayInput::Modal,
            Some(dismiss),
            window,
            cx,
        );
        let bounds_state = window.use_keyed_state((self.id.clone(), "geometry"), cx, |_, _| {
            (Bounds::<Pixels>::default(), None)
        });
        let pointer_state = bounds_state.clone();
        let trigger_state = state.clone();
        div()
            .id(self.id)
            .key_context("Popover")
            .on_prepaint(move |bounds, _, cx| bounds_state.update(cx, |state, _| state.0 = bounds))
            .on_action({
                let state = state.clone();
                move |_: &gpui_base::actions::Confirm, window, cx| {
                    state.update(cx, |state, cx| state.toggle_open(window, cx));
                    cx.notify(parent);
                }
            })
            .on_mouse_down(self.mouse_button, move |event, window, cx| {
                pointer_state.update(cx, |geometry, _| geometry.1 = Some(event.position));
                trigger_state.update(cx, |state, cx| state.toggle_open(window, cx));
                window.prevent_default();
                cx.stop_propagation();
                cx.notify(parent);
            })
            .child(trigger(open))
            .into_any_element()
    }
}

use gpui::prelude::FluentBuilder as _;
#[derive(IntoElement)]
struct Trigger {
    build: Box<dyn FnOnce(bool) -> AnyElement>,
    open: bool,
}
impl Selectable for Trigger {
    fn selected(mut self, open: bool) -> Self {
        self.open = open;
        self
    }
    fn is_selected(&self) -> bool {
        self.open
    }
}
impl RenderOnce for Trigger {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        (self.build)(self.open)
    }
}

type MenuBuilder = Rc<
    dyn Fn(
        gpui_component::menu::PopupMenu,
        &mut Window,
        &mut Context<gpui_component::menu::PopupMenu>,
    ) -> gpui_component::menu::PopupMenu,
>;

pub(crate) trait DropdownMenu:
    Styled + Selectable + gpui::InteractiveElement + gpui::ParentElement + IntoElement + Sized + 'static
{
    fn dropdown_menu(
        self,
        build: impl Fn(
            gpui_component::menu::PopupMenu,
            &mut Window,
            &mut Context<gpui_component::menu::PopupMenu>,
        ) -> gpui_component::menu::PopupMenu
        + 'static,
    ) -> Menu<Self> {
        self.dropdown_menu_with_anchor(Anchor::TopLeft, build)
    }
    fn dropdown_menu_with_anchor(
        mut self,
        anchor: Anchor,
        build: impl Fn(
            gpui_component::menu::PopupMenu,
            &mut Window,
            &mut Context<gpui_component::menu::PopupMenu>,
        ) -> gpui_component::menu::PopupMenu
        + 'static,
    ) -> Menu<Self> {
        let id = self.interactivity().element_id.clone().unwrap_or(0.into());
        Menu {
            id: SharedString::from(format!("dropdown-menu:{id:?}")).into(),
            trigger: self,
            anchor,
            context: false,
            build: Rc::new(build),
        }
    }
}
impl<
    T: Styled + Selectable + gpui::InteractiveElement + gpui::ParentElement + IntoElement + 'static,
> DropdownMenu for T
{
}

pub(crate) trait ContextMenuExt: DropdownMenu + gpui::ParentElement {
    fn context_menu(
        mut self,
        build: impl Fn(
            gpui_component::menu::PopupMenu,
            &mut Window,
            &mut Context<gpui_component::menu::PopupMenu>,
        ) -> gpui_component::menu::PopupMenu
        + 'static,
    ) -> Menu<Self> {
        let id = self
            .interactivity()
            .element_id
            .clone()
            .expect("app context menus have stable ids");
        Menu {
            id: SharedString::from(format!("context-menu:{id:?}")).into(),
            trigger: self,
            anchor: Anchor::TopLeft,
            context: true,
            build: Rc::new(build),
        }
    }
}
impl<T: DropdownMenu + gpui::ParentElement> ContextMenuExt for T {}

#[derive(IntoElement)]
pub(crate) struct Menu<T: DropdownMenu> {
    id: ElementId,
    trigger: T,
    anchor: Anchor,
    context: bool,
    build: MenuBuilder,
}
impl<T: DropdownMenu> Styled for Menu<T> {
    fn style(&mut self) -> &mut StyleRefinement {
        self.trigger.style()
    }
}
impl<T: DropdownMenu> gpui::ParentElement for Menu<T> {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.trigger.extend(elements);
    }
}

#[derive(Default)]
struct MenuState {
    menu: Option<gpui::Entity<gpui_component::menu::PopupMenu>>,
    subscription: Option<gpui::Subscription>,
}

impl<T: DropdownMenu> RenderOnce for Menu<T> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if !popup::uses_native_popups(window, cx) {
            if self.context {
                return gpui_component::menu::ContextMenuExt::context_menu(
                    self.trigger,
                    move |menu, window, cx| (self.build)(menu, window, cx),
                )
                .into_any_element();
            }
            let menu = gpui_component::menu::DropdownMenu::dropdown_menu_with_anchor(
                FallbackTrigger(self.trigger),
                self.anchor,
                move |menu, window, cx| (self.build)(menu, window, cx),
            );
            return menu.into_any_element();
        }
        let menu_state =
            window.use_keyed_state((self.id.clone(), "menu"), cx, |_, _| MenuState::default());
        let owner_window = window.window_handle();
        let build = self.build;
        let mut popover = Popover::new(self.id)
            .anchor(self.anchor)
            .appearance(false)
            .trigger(self.trigger)
            .on_open_change({
                let state = menu_state.clone();
                move |open, window, cx| {
                    if !*open {
                        state.update(cx, |state, _| {
                            state.menu = None;
                            state.subscription = None;
                        });
                    }
                    let _ = window;
                }
            })
            .content(move |_, window, cx| {
                if let Some(menu) = menu_state.read(cx).menu.clone() {
                    return menu;
                }
                let menu =
                    gpui_component::menu::PopupMenu::build(window, cx, |menu, window, cx| {
                        build(menu, window, cx)
                    });
                menu.focus_handle(cx).focus(window, cx);
                let popover = cx.entity().downgrade();
                let subscription = cx.subscribe(&menu, move |_, _, _: &gpui::DismissEvent, cx| {
                    let popover = popover.clone();
                    cx.defer(move |cx| {
                        let _ = owner_window.update(cx, |_, window, cx| {
                            let _ = popover.update(cx, |state, cx| state.dismiss(window, cx));
                            window.refresh();
                        });
                    });
                });
                menu_state.update(cx, |state, _| {
                    state.menu = Some(menu.clone());
                    state.subscription = Some(subscription);
                });
                menu
            });
        if self.context {
            popover.mouse_button = MouseButton::Right;
            popover.at_pointer = true;
        }
        popover.into_any_element()
    }
}

#[derive(IntoElement)]
struct FallbackTrigger<T: IntoElement + 'static>(T);
impl<T: Styled + IntoElement + 'static> Styled for FallbackTrigger<T> {
    fn style(&mut self) -> &mut StyleRefinement {
        self.0.style()
    }
}
impl<T: Selectable + IntoElement + 'static> Selectable for FallbackTrigger<T> {
    fn selected(mut self, selected: bool) -> Self {
        self.0 = self.0.selected(selected);
        self
    }
    fn is_selected(&self) -> bool {
        self.0.is_selected()
    }
}
impl<T: gpui::InteractiveElement + IntoElement + 'static> gpui::InteractiveElement
    for FallbackTrigger<T>
{
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.0.interactivity()
    }
}
impl<T: IntoElement + 'static> RenderOnce for FallbackTrigger<T> {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.0
    }
}
impl<T: Styled + Selectable + gpui::InteractiveElement + IntoElement + 'static>
    gpui_component::menu::DropdownMenu for FallbackTrigger<T>
{
}
