pub(in crate::app::views) mod native;

use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, FocusHandle, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, Render, Styled as _, Subscription, WeakEntity, Window,
    WindowBounds, WindowHandle, WindowOptions, div, point, px,
};

use super::FarcasterApp;
use crate::app::{
    APP_INPUT_CONTEXT, AppSurface, OVERLAY_KEY_CONTEXT,
    ui::{
        primitives::modal,
        theme::{theme, ui_font},
    },
};
use native::{NativeHost, OverlayInput};

#[derive(Default)]
pub(in crate::app) struct ModalWindowState {
    pub(in crate::app) window: Option<WindowHandle<ModalWindow>>,
    owner_window: Option<AnyWindowHandle>,
    pending: bool,
    failed: bool,
    observers: Vec<Subscription>,
}

pub(in crate::app) struct ModalWindow {
    owner: WeakEntity<FarcasterApp>,
    host: NativeHost,
    focus: Option<FocusHandle>,
    scope: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl FarcasterApp {
    pub(in crate::app) fn modal_uses_native_host(&self) -> bool {
        // An ungrabbed Wayland popup cannot guarantee keyboard focus for an async request.
        // Keep the existing path there until GPUI can host a modal without a new tile.
        cfg!(target_os = "macos")
            && !self.overlays.modal.failed
            && matches!(
                self.workspace.surface,
                AppSurface::Editor | AppSurface::Terminal
            )
            && !self.native_workspace_other_modal_active()
            && match (&self.workspace.send_to_chat, &self.extensions.active.dialog) {
                (Some(_), None) => true,
                (None, Some(dialog)) => !matches!(
                    dialog,
                    crate::protocol::ExtensionUiRequest::WorkerModel { .. }
                ),
                _ => false,
            }
    }

    pub(in crate::app) fn modal_rendered_in_native_window(&self) -> bool {
        self.modal_uses_native_host()
            && (self.overlays.modal.window.is_some() || self.overlays.modal.pending)
    }

    pub(in crate::app) fn modal_owner_window(&self, window: &Window) -> Option<AnyWindowHandle> {
        self.overlays
            .modal
            .window
            .filter(|popup| AnyWindowHandle::from(*popup) == window.window_handle())
            .and(self.overlays.modal.owner_window)
    }

    pub(in crate::app) fn prepare_modal_window(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlays.modal.observers.is_empty() {
            self.overlays.modal.observers = vec![
                cx.observe_window_bounds(window, |_, _, cx| cx.notify()),
                cx.observe_window_activation(window, |_, _, cx| cx.notify()),
                cx.observe_window_visibility(window, |_, _, _, cx| cx.notify()),
                cx.on_release(|this, cx| {
                    if let Some(popup) = this.overlays.modal.window.take() {
                        let _ = popup.update(cx, |_, window, _| window.remove_window());
                    }
                }),
            ];
        }
        if !self.native_workspace_covered_by_overlay() {
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

    fn render_native_modal(&self, scope: &FocusHandle, cx: &Context<Self>) -> gpui::AnyElement {
        let owner = cx.entity().downgrade();
        if self.workspace.send_to_chat.is_some() {
            return super::dialogs::send_to_chat::render(self, owner, cx);
        }
        let dismiss = owner.clone();
        modal(
            "native-extension-dialog",
            "Agent request",
            scope,
            OVERLAY_KEY_CONTEXT,
            move |window, cx| {
                let _ = dismiss.update(cx, |app, cx| app.cancel_dialog(window, cx));
            },
            |surface| surface.child(self.render_composer_request(owner, true)),
        )
        .into_any_element()
    }
}

fn synchronize(owner: WeakEntity<FarcasterApp>, window: &mut Window, cx: &mut App) {
    let Some(app) = owner.upgrade() else { return };
    let existing = app.read(cx).overlays.modal.window;
    let popup_active = existing.is_some_and(|popup| {
        popup
            .update(cx, |_, window, _| window.is_window_active())
            .unwrap_or(false)
    });
    let wanted = app.read(cx).modal_uses_native_host()
        // Retain an existing panel through focus handoffs and app deactivation.
        // AppKit hides it with the app; removing it while the owner resigns key
        // can race the panel's activation callback and discard a focused input.
        && (window.is_window_active() || existing.is_some())
        && window.visibility() == gpui::WindowVisibility::Visible;
    let bounds = Bounds::new(window.bounds().origin, window.viewport_size());
    let owner_active = window.is_window_active();
    let result = if wanted {
        if let Some(popup) = existing {
            popup
                .update(cx, |view, window, cx| {
                    if window.viewport_size() != bounds.size {
                        window.resize(bounds.size);
                    }
                    view.host
                        .position(window, bounds, point(px(0.0), px(0.0)))?;
                    if !window.is_window_active() && owner_active {
                        view.host.present(
                            window,
                            bounds,
                            point(px(0.0), px(0.0)),
                            OverlayInput::Modal,
                        )?;
                    }
                    cx.notify();
                    Ok(popup)
                })
                .map_err(|error| error.to_string())
                .and_then(|result| result)
        } else {
            open(owner, window, bounds, cx)
        }
        .map(Some)
    } else {
        if let Some(popup) = existing {
            let _ = popup.update(cx, |_, window, _| window.remove_window());
            // Closing a modal should not activate the app after the user switched away.
            if popup_active && window.visibility() == gpui::WindowVisibility::Visible {
                window.activate_window();
            }
        }
        Ok(None)
    };
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
                    let _ = popup.update(cx, |_, window, _| window.remove_window());
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
    cx: &mut App,
) -> Result<WindowHandle<ModalWindow>, String> {
    let host = NativeHost::capture(window)?;
    let options = WindowOptions {
        kind: gpui::WindowKind::PopUp,
        focus: false,
        show: false,
        titlebar: None,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        window_background: gpui::WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let popup = cx
        .open_window(options, move |window, cx| {
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
                ModalWindow {
                    owner,
                    host,
                    focus: None,
                    scope: cx.focus_handle(),
                    _subscriptions: subscriptions,
                }
            })
        })
        .map_err(|error| error.to_string())?;
    let result = popup
        .update(cx, |view, window, _| {
            view.host
                .present(window, bounds, point(px(0.0), px(0.0)), OverlayInput::Modal)
        })
        .map_err(|error| error.to_string())
        .and_then(|result| result);
    if let Err(error) = result {
        let _ = popup.update(cx, |_, window, _| window.remove_window());
        return Err(error);
    }
    Ok(popup)
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
            let focus = app
                .keyboard_overlay_focus(window, cx)
                .unwrap_or_else(|| app.composer_region_focus(cx));
            if self.focus.as_ref() != Some(&focus) {
                focus.focus(window, cx);
                self.focus = Some(focus);
            }
            let root = div()
                .relative()
                .size_full()
                .font(ui_font())
                .text_color(theme().colors.text)
                .text_size(theme().type_scale.body)
                .key_context(APP_INPUT_CONTEXT);
            super::root::bind_overlay_actions(root, cx)
                .child(app.render_native_modal(&self.scope, cx))
                .children(app.has_floating_notices().then(|| {
                    app.render_floating_notices(
                        cx.entity().downgrade(),
                        app.overlays.modal.owner_window,
                    )
                }))
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
