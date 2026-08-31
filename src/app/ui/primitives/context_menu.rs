use gpui::{
    AnyElement, Div, InteractiveElement as _, Interactivity, IntoElement, ParentElement as _,
    RenderOnce, Stateful, StyleRefinement, Styled, div,
};
use gpui_component::{Selectable, StyledExt as _};

#[derive(IntoElement)]
pub(crate) struct ContextMenuTrigger {
    base: Stateful<Div>,
    selected: bool,
    style: StyleRefinement,
}

impl ContextMenuTrigger {
    pub(crate) fn new(id: impl Into<gpui::ElementId>, child: AnyElement) -> Self {
        Self {
            base: div().id(id).w_full().child(child),
            selected: false,
            style: StyleRefinement::default(),
        }
    }
}

impl Selectable for ContextMenuTrigger {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl Styled for ContextMenuTrigger {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl gpui::InteractiveElement for ContextMenuTrigger {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl gpui::StatefulInteractiveElement for ContextMenuTrigger {}

impl RenderOnce for ContextMenuTrigger {
    fn render(self, _: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
        self.base.refine_style(&self.style)
    }
}

impl gpui_component::menu::DropdownMenu for ContextMenuTrigger {}
