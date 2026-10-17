pub(in crate::app) mod keyboard;
pub(in crate::app) mod native;
pub(in crate::app) mod panel;
pub(in crate::app) mod popup;
pub(in crate::app::views) mod stack;
#[cfg(target_os = "linux")]
mod x11;

use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, Render, Styled as _, Subscription, WeakEntity, Window,
    div, point, px,
};

use super::FarcasterApp;
use crate::app::{
    APP_INPUT_CONTEXT, AppSurface, OVERLAY_KEY_CONTEXT,
    ui::{
        primitives::modal,
        theme::{theme, ui_font},
    },
};
use gpui_base::ElementExt as _;
use native::{NativeHost, OverlayInput};
use panel::Panel;

#[derive(Default)]
pub(in crate::app) struct ModalWindowState {
    pub(in crate::app) window: Option<Panel<ModalWindow>>,
    pub(in crate::app::views) owner_window: Option<AnyWindowHandle>,
    pending: bool,
    failed: bool,
    passive_height: Option<Pixels>,
    observers: Vec<Subscription>,
}

pub(in crate::app) struct ModalWindow {
    owner: WeakEntity<FarcasterApp>,
    host: NativeHost,
    input: OverlayInput,
    owner_size: gpui::Size<Pixels>,
    owner_left: Pixels,
    focus: Option<FocusHandle>,
    scope: FocusHandle,
    pub(in crate::app) tooltips: gpui::Entity<crate::app::ui::primitives::AppTooltips>,
    _subscriptions: Vec<Subscription>,
}

impl FarcasterApp {
    pub(in crate::app) fn native_overlays_available(&self) -> bool {
        cfg!(any(target_os = "macos", target_os = "linux")) && !self.overlays.modal.failed
    }

    pub(in crate::app::views) fn native_request_is_passive(&self) -> bool {
        !self.native_workspace_modal_active()
            && self
                .extensions
                .active
                .dialog
                .as_ref()
                .is_some_and(|request| {
                    !request.is_blocking()
                        || matches!(
                            request,
                            crate::protocol::ExtensionUiRequest::WorkerModel { .. }
                        )
                })
            && !self.workspace.runtime_picker.open
    }

    fn modal_placement(
        &self,
        window: &Window,
        cx: &App,
    ) -> (Bounds<Pixels>, gpui::Point<Pixels>, OverlayInput) {
        let viewport = window.viewport_size();
        if !self.native_request_is_passive() {
            return (
                Bounds::new(window.bounds().origin, viewport),
                point(px(0.0), px(0.0)),
                OverlayInput::Modal,
            );
        }
        let (left, width) = self.native_workspace_width(viewport.width, cx);
        let height = self
            .overlays
            .modal
            .passive_height
            .unwrap_or(theme().size(160.0))
            .min((viewport.height - theme().size(38.0)).max(px(1.0)));
        let offset = point(left, viewport.height - height);
        (
            Bounds::new(window.bounds().origin + offset, gpui::size(width, height)),
            offset,
            OverlayInput::Passive,
        )
    }

    fn native_workspace_width(&self, width: Pixels, cx: &App) -> (Pixels, Pixels) {
        use crate::app::ui::layout::{layout_mode, shows_left_inline, shows_right_inline};
        let mode = layout_mode(width);
        let left = if shows_left_inline(mode) && !self.workspace.session_rail_hidden {
            self.views.session_rail.read(cx).width()
        } else {
            px(0.0)
        };
        let right = if shows_right_inline(mode) && !self.workspace.run_panel_hidden {
            self.views.run_panel.read(cx).width()
        } else {
            px(0.0)
        };
        (left, (width - left - right).max(px(1.0)))
    }

    pub(in crate::app) fn modal_uses_native_host(&self) -> bool {
        self.native_overlays_available()
            && matches!(
                self.workspace.surface,
                AppSurface::Editor | AppSurface::Terminal
            )
            && (self.native_workspace_modal_active() || self.extensions.active.dialog.is_some())
    }

    pub(in crate::app) fn modal_rendered_in_native_window(&self) -> bool {
        self.modal_uses_native_host()
            && (self.overlays.modal.window.is_some() || self.overlays.modal.pending)
    }

    pub(in crate::app) fn modal_owner_window(
        &self,
        window: &Window,
        cx: &App,
    ) -> Option<AnyWindowHandle> {
        self.overlays
            .modal
            .window
            .filter(|popup| AnyWindowHandle::from(*popup) == window.window_handle())
            .and(self.overlays.modal.owner_window)
            .or_else(|| {
                Panel::<popup::PopupWindow>::from_window(window, cx)
                    .map(|view| view.read(cx).owner_window)
            })
    }

    pub(in crate::app) fn route_overlay_action(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> bool {
        let Some(owner) = self.modal_owner_window(window, cx) else {
            return false;
        };
        in_owner_window(cx.entity().downgrade(), Some(owner), window, cx, action);
        true
    }

    pub(in crate::app) fn prepare_modal_window(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlays.modal.observers.is_empty() {
            let id = window.window_handle().window_id();
            popup::register_owner(window, cx.entity().downgrade(), cx);
            self.overlays.modal.observers = vec![
                cx.observe_window_bounds(window, |_, _, cx| cx.notify()),
                cx.observe_window_activation(window, |_, _, cx| cx.notify()),
                cx.observe_window_visibility(window, |_, _, _, cx| cx.notify()),
                cx.on_release(move |this, cx| {
                    popup::release_owner(id, cx);
                    if let Some(popup) = this.overlays.modal.window.take() {
                        let _ = popup.update(cx, |_, window, cx| stack::close(window, cx));
                    }
                }),
            ];
        }
        if !self.native_workspace_covered_by_overlay()
            && self.extensions.active.dialog.is_none()
            && !gpui_base::GlobalState::is_in_deferred_context(cx)
            && !crate::app::ui::primitives::tooltip_overlay(window, cx)
                .is_some_and(|tooltips| tooltips.read(cx).is_visible())
        {
            self.overlays.modal.failed = false;
        }
        if self.overlays.modal.pending
            || (!self.modal_uses_native_host() && self.overlays.modal.window.is_none())
        {
            return;
        }
        self.overlays.modal.pending = true;
        self.overlays.modal.owner_window = Some(window.window_handle());
        let owner = cx.entity().downgrade();
        window.defer(cx, move |window, cx| synchronize(owner, window, cx));
    }
}

fn synchronize(owner: WeakEntity<FarcasterApp>, window: &mut Window, cx: &mut App) {
    let Some(app) = owner.upgrade() else { return };
    let mut existing = app.read(cx).overlays.modal.window;
    existing = existing.filter(|popup| cx.windows().contains(&(*popup).into()));
    let popup_active = existing.is_some_and(|popup| {
        popup
            .update(cx, |_, window, _| window.is_window_active())
            .unwrap_or(false)
    });
    let hides_on_deactivate = existing.is_some_and(|popup| {
        popup
            .read(cx)
            .is_ok_and(|view| view.host.hides_on_deactivate())
    });
    let wanted = app.read(cx).modal_uses_native_host()
        // Retain an existing panel through focus handoffs and app deactivation.
        // AppKit hides it with the app; removing it while the owner resigns key
        // can race the panel's activation callback and discard a focused input.
        && (popup::owner_active(window, cx) || popup_active || hides_on_deactivate)
        && window.visibility() == gpui::WindowVisibility::Visible;
    let (bounds, offset, input) = app.read(cx).modal_placement(window, cx);
    let was_modal = existing.is_some_and(|popup| {
        popup
            .read(cx)
            .is_ok_and(|view| view.input == OverlayInput::Modal)
    });
    let owner_active = window.is_window_active();
    if wanted
        && existing.is_some_and(|popup| {
            popup.read(cx).is_ok_and(|view| {
                (view.host.relays_input(view.input) || view.host.relays_input(input))
                    && (view.owner_size != window.viewport_size()
                        || view.owner_left != offset.x
                        || view.input != input)
            })
        })
    {
        let popup = existing.take().unwrap();
        let _ = popup.update(cx, |_, window, cx| stack::close(window, cx));
    }
    let result = if wanted {
        if let Some(popup) = existing {
            popup
                .update(cx, |view, window, cx| {
                    if window.bounds().size != bounds.size {
                        window.resize(bounds.size);
                    }
                    view.host.position(window, bounds, offset)?;
                    if view.input != input
                        || (!view.host.is_wayland() && !window.is_window_active() && owner_active)
                    {
                        view.host.present(window, bounds, offset, input)?;
                        view.input = input;
                        cx.notify();
                    }
                    Ok(popup)
                })
                .map_err(|error| error.to_string())
                .and_then(|result| result)
        } else {
            open(owner, window, bounds, offset, input, cx)
        }
        .map(Some)
    } else {
        if let Some(popup) = existing {
            let _ = popup.update(cx, |_, window, cx| stack::close(window, cx));
            // Closing a modal should not activate the app after the user switched away.
            if popup_active && window.visibility() == gpui::WindowVisibility::Visible {
                window.activate_window();
            }
        }
        Ok(None)
    };
    if wanted && was_modal && input == OverlayInput::Passive {
        window.activate_window();
        app.update(cx, |app, _| app.request_active_surface_focus(None));
    }
    app.update(cx, |app, cx| {
        app.overlays.modal.pending = false;
        match result {
            Ok(next) => {
                if app.overlays.modal.window != next {
                    app.overlays.modal.window = next;
                    cx.notify();
                }
            }
            Err(error) => {
                if let Some(popup) = existing {
                    let _ = popup.update(cx, |_, window, cx| stack::close(window, cx));
                }
                app.overlays.modal.window = None;
                app.overlays.modal.failed = true;
                zlog::warn!("Native modal window: {error}");
                cx.notify();
            }
        }
    });
}

fn open(
    owner: WeakEntity<FarcasterApp>,
    window: &Window,
    bounds: Bounds<Pixels>,
    offset: gpui::Point<Pixels>,
    input: OverlayInput,
    cx: &mut App,
) -> Result<Panel<ModalWindow>, String> {
    let owner_size = window.viewport_size();
    NativeHost::open_panel(
        window,
        bounds,
        input,
        offset,
        move |host, window, cx| {
            cx.new(|cx| {
                let mut subscriptions = Vec::new();
                if let Some(app) = owner.upgrade() {
                    subscriptions.push(cx.observe(&app, |_, _, cx| cx.notify()));
                }
                let window_id = window.window_handle().window_id();
                let event_owner = owner.clone();
                // The app's Escape prefix interceptor only belongs to its main window.
                // Resolve modal Escape before the global sequence matcher buffers it.
                subscriptions.push(cx.intercept_keystrokes(move |event, window, cx| {
                    if window.window_handle().window_id() == window_id
                        && event.keystroke.key == "escape"
                        && !event.keystroke.modifiers.modified()
                    {
                        let _ = event_owner.update(cx, |app, cx| app.dismiss_surface(window, cx));
                        cx.stop_propagation();
                    }
                }));
                let tooltips = cx.new(|_| crate::app::ui::primitives::AppTooltips::default());
                subscriptions.push(cx.observe(&tooltips, |_, _, cx| cx.notify()));
                ModalWindow {
                    tooltips,
                    owner,
                    host,
                    input,
                    owner_size,
                    owner_left: offset.x,
                    focus: None,
                    scope: cx.focus_handle(),
                    _subscriptions: subscriptions,
                }
            })
        },
        cx,
    )
}

impl Render for ModalWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(owner) = self.owner.upgrade() else {
            return div().into_any_element();
        };
        owner.update(cx, |app, cx| {
            if !app.modal_uses_native_host() {
                return div().into_any_element();
            }
            if app.native_request_is_passive() {
                self.focus = None;
                let measure = cx.entity().downgrade();
                let scope = self.scope.clone();
                let relays_input = self.host.relays_input(self.input);
                return div()
                    .track_focus(&self.scope)
                    .font(ui_font())
                    .text_color(theme().colors.text)
                    .text_size(theme().type_scale.body)
                    .child(app.render_composer_request(cx.entity().downgrade(), false))
                    .on_prepaint(move |bounds, _, cx| {
                        let _ = measure.update(cx, |app, cx| {
                            let height = bounds.size.height;
                            if app.overlays.modal.passive_height != Some(height) {
                                app.overlays.modal.passive_height = Some(height);
                                cx.notify();
                            }
                        });
                    })
                    .child(gpui::canvas(
                        |_, _, _| (),
                        move |_, _, window, cx| {
                            if relays_input {
                                keyboard::set_passive_focus(
                                    window.window_handle(),
                                    scope.contains_focused(window, cx),
                                    cx,
                                );
                            }
                        },
                    ))
                    .child(self.tooltips.clone())
                    .into_any_element();
            }
            let focus = app
                .keyboard_overlay_focus(window, cx)
                .unwrap_or_else(|| app.composer_region_focus(cx));
            let initial_focus = (self.focus.as_ref() != Some(&focus)).then_some(focus.clone());
            if initial_focus.is_some() && self.host.is_wayland() {
                focus.focus(window, cx);
            }
            self.focus = Some(focus);
            let root = div()
                .relative()
                .size_full()
                .font(ui_font())
                .text_color(theme().colors.text)
                .text_size(theme().type_scale.body)
                .key_context(APP_INPUT_CONTEXT);
            let owner = cx.entity().downgrade();
            let picker = app.render_picker(owner.clone(), cx);
            let root = super::root::bind_overlay_actions(root, cx);
            let root = if matches!(
                app.extensions.active.dialog,
                Some(crate::protocol::ExtensionUiRequest::WorkerModel { .. })
            ) {
                let (left, width) = app.native_workspace_width(window.viewport_size().width, cx);
                let dismiss = owner.clone();
                root.on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                    let _ = dismiss
                        .update(cx, |app, cx| app.set_runtime_picker_open(false, window, cx));
                    cx.stop_propagation();
                })
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left(left)
                        .w(width)
                        .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
                        .child(app.render_composer_request(owner.clone(), true)),
                )
            } else if app.extensions.active.dialog.is_some() {
                let dismiss = owner.clone();
                root.child(modal(
                    "native-extension-dialog",
                    "Agent request",
                    &self.scope,
                    OVERLAY_KEY_CONTEXT,
                    move |window, cx| {
                        let _ = dismiss.update(cx, |app, cx| {
                            app.cancel_dialog(window, cx);
                        });
                    },
                    |surface| surface.child(app.render_composer_request(owner.clone(), true)),
                ))
            } else {
                root
            };
            app.render_overlay_contents(root, owner, picker, false, cx)
                .child(self.tooltips.clone())
                .on_prepaint(move |_, window, cx| {
                    if let Some(focus) = initial_focus {
                        focus.focus(window, cx);
                    }
                })
                .into_any_element()
        })
    }
}

pub(in crate::app) fn in_owner_window(
    owner: WeakEntity<FarcasterApp>,
    owner_window: Option<AnyWindowHandle>,
    window: &mut Window,
    cx: &mut App,
    action: impl FnOnce(&mut FarcasterApp, &mut Window, &mut Context<FarcasterApp>) + 'static,
) {
    if let Some(target) = owner_window.filter(|target| *target != window.window_handle()) {
        cx.defer(move |cx| {
            let _ = target.update(cx, |_, window, cx| {
                let _ = owner.update(cx, |app, cx| action(app, window, cx));
            });
        });
    } else {
        let _ = owner.update(cx, |app, cx| action(app, window, cx));
    }
}

#[cfg(test)]
#[path = "../native_overlay_tests.rs"]
mod tests;
