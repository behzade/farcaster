use crate::{
    ActiveTheme, StyledExt,
    input::{AnyInputState, Copy},
    native_menu::FallbackMenuOverlay,
    tooltip::render_tooltip,
    window_border,
};
use gpui::{
    AnyView, App, AppContext, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement,
    KeyBinding, ParentElement as _, Pixels, Render, StyleRefinement, Styled, Window, actions, div,
};
use gpui_base::{TextSelection, TextSelectionLayer, TextSelectionScopeId};

actions!(root, [Tab, TabPrev]);

const CONTEXT: &str = "Root";

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", Tab, Some(CONTEXT)),
        KeyBinding::new("shift-tab", TabPrev, Some(CONTEXT)),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-c", Copy, Some(CONTEXT)),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-c", Copy, Some(CONTEXT)),
    ]);
}

/// Root behavior retained by Pi: app content, selection, input tracking,
/// tooltips, native input menus, focus navigation, and window borders.
pub struct Root {
    style: StyleRefinement,
    view: AnyView,
    pub(super) focused_input: Option<AnyInputState>,
    pub(crate) tooltip_overlay: Entity<gpui_base::TooltipOverlay>,
    pub(crate) native_menu_overlay: Entity<FallbackMenuOverlay>,
    window_shadow_size: Pixels,
    bordered: bool,
    window_id: gpui::WindowId,
}

impl Root {
    #[deprecated(note = "use gpui_base::TextSelection::clear instead")]
    pub fn clear_text_selection(&mut self, cx: &mut Context<Self>) {
        gpui_base::TextSelection::clear_for_window(self.window_id, cx);
    }

    pub fn new(view: impl Into<AnyView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        #[cfg(all(target_os = "macos", not(test)))]
        gpui_base::install_window_hit_test_forwarder(window);

        Self {
            style: StyleRefinement::default(),
            view: view.into(),
            focused_input: None,
            tooltip_overlay: cx
                .new(|_| gpui_base::TooltipOverlay::new().render_with(render_tooltip)),
            native_menu_overlay: cx.new(|_| FallbackMenuOverlay::new()),
            window_shadow_size: window_border::SHADOW_SIZE,
            bordered: true,
            window_id: window.window_handle().window_id(),
        }
    }

    pub(crate) fn active_text_selection_scope(&self) -> TextSelectionScopeId {
        TextSelectionScopeId::default()
    }

    pub fn bordered(mut self, bordered: bool) -> Self {
        self.bordered = bordered;
        self
    }

    pub fn window_shadow_size(mut self, size: impl Into<Pixels>) -> Self {
        self.window_shadow_size = size.into();
        self
    }

    pub fn update<F, R>(window: &mut Window, cx: &mut App, f: F) -> R
    where
        F: FnOnce(&mut Self, &mut Window, &mut Context<Self>) -> R,
    {
        let root = window
            .root::<Root>()
            .flatten()
            .expect("BUG: window first layer should be a gpui_component::Root.");
        root.update(cx, |root, cx| f(root, window, cx))
    }

    pub(crate) fn try_update<F, R>(window: &mut Window, cx: &mut App, f: F) -> Option<R>
    where
        F: FnOnce(&mut Self, &mut Window, &mut Context<Self>) -> R,
    {
        let root = window.root::<Root>().flatten()?;
        Some(root.update(cx, |root, cx| f(root, window, cx)))
    }

    pub fn read<'a>(window: &'a Window, cx: &'a App) -> &'a Self {
        window
            .root::<Root>()
            .expect("The window root view should be of type gpui_component::Root.")
            .expect("The gpui_component::Root entity should be available.")
            .read(cx)
    }

    pub(crate) fn tooltip_overlay(
        window: &Window,
        cx: &App,
    ) -> Option<Entity<gpui_base::TooltipOverlay>> {
        let root = window.root::<Root>()??;
        Some(root.read(cx).tooltip_overlay.clone())
    }

    pub(crate) fn native_menu_overlay(
        window: &Window,
        cx: &App,
    ) -> Option<Entity<FallbackMenuOverlay>> {
        let root = window.root::<Root>()??;
        Some(root.read(cx).native_menu_overlay.clone())
    }

    pub fn view(&self) -> &AnyView {
        &self.view
    }

    fn on_action_tab(&mut self, _: &Tab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(container) = gpui_base::active_focus_trap(window, cx) {
            let before = window.focused(cx);
            window.focus_next(cx);
            for _ in 0..100 {
                if container.contains_focused(window, cx) || window.focused(cx) == before {
                    break;
                }
                window.focus_next(cx);
            }
            return;
        }
        window.focus_next(cx);
    }

    fn on_action_tab_prev(&mut self, _: &TabPrev, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(container) = gpui_base::active_focus_trap(window, cx) {
            let before = window.focused(cx);
            window.focus_prev(cx);
            for _ in 0..100 {
                if container.contains_focused(window, cx) || window.focused(cx) == before {
                    break;
                }
                window.focus_prev(cx);
            }
            return;
        }
        window.focus_prev(cx);
    }

    fn on_action_copy(&mut self, _: &Copy, window: &mut Window, cx: &mut Context<Self>) {
        let text = TextSelection::selected_text(window, cx).trim().to_string();
        if text.is_empty() {
            cx.propagate();
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
}

impl Styled for Root {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl Render for Root {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        window.set_rem_size(cx.theme().font_size);
        if !cx.has_global::<crate::global_state::UiGlobalState>() {
            crate::global_state::init(cx);
        }
        crate::global_state::UiGlobalState::global_mut(cx).begin_selection_frame();
        TextSelection::activate_scope(self.active_text_selection_scope(), window, cx);

        let inner = div()
            .id("root")
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::on_action_tab))
            .on_action(cx.listener(Self::on_action_tab_prev))
            .on_action(cx.listener(Self::on_action_copy))
            .relative()
            .size_full()
            .font_family(cx.theme().font_family.clone())
            .bg(cx.theme().tokens.background)
            .text_color(cx.theme().foreground)
            .refine_style(&self.style)
            .child(TextSelectionLayer)
            .child(self.view.clone())
            .child(self.tooltip_overlay.clone())
            .child(self.native_menu_overlay.clone());

        if self.bordered {
            window_border()
                .shadow_size(self.window_shadow_size)
                .child(inner)
                .into_any_element()
        } else {
            inner.into_any_element()
        }
    }
}
