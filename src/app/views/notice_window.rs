use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, Pixels, Render, Size, StatefulInteractiveElement as _, Styled as _,
    Subscription, WeakEntity, Window, WindowBounds, WindowHandle, WindowOptions, div, point, px,
    size,
};

use super::FarcasterApp;
use super::native_overlay::native::{NativeHost, OverlayInput};
use crate::app::{
    AppSurface,
    ui::theme::{theme, ui_font},
};
use gpui_base::ElementExt as _;

fn window_inset() -> Pixels {
    theme().space.md
}

#[derive(Default)]
pub(in crate::app) struct NoticeWindowState {
    pub(in crate::app) window: Option<WindowHandle<NoticeWindow>>,
    pending: bool,
    failed: bool,
    observers: Vec<Subscription>,
}

pub(in crate::app) struct NoticeWindow {
    owner: WeakEntity<FarcasterApp>,
    owner_window: AnyWindowHandle,
    owner_bounds: Bounds<Pixels>,
    owner_size: Size<Pixels>,
    host: NativeHost,
}

impl FarcasterApp {
    pub(in crate::app) fn has_floating_notices(&self) -> bool {
        !self.extensions.active.notifications.is_empty()
            || self.workspace.code_tasks.notice_message().is_some()
    }

    fn notices_need_native_window(&self) -> bool {
        self.has_floating_notices()
            && matches!(
                self.workspace.surface,
                AppSurface::Editor | AppSurface::Terminal
            )
            && !self.native_workspace_covered_by_overlay()
            && (!self.modal_rendered_in_native_window() || self.native_request_is_passive())
    }

    pub(in crate::app) fn notices_render_in_native_window(&self) -> bool {
        self.notices_need_native_window()
            && (self.overlays.notices.window.is_some() || self.overlays.notices.pending)
            && !self.overlays.notices.failed
    }

    pub(in crate::app) fn prepare_notice_window(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.overlays.notices.observers.is_empty() {
            self.overlays.notices.observers = vec![
                cx.observe_window_bounds(window, |_, _, cx| cx.notify()),
                cx.observe_window_activation(window, |_, _, cx| cx.notify()),
                cx.observe_window_visibility(window, |_, _, _, cx| cx.notify()),
                cx.on_release(|this, cx| {
                    if let Some(popup) = this.overlays.notices.window.take() {
                        let _ = popup.update(cx, |_, window, cx| {
                            super::native_overlay::stack::close(window, cx)
                        });
                    }
                }),
            ];
        }
        let needed = self.notices_need_native_window();
        if !needed {
            self.overlays.notices.failed = false;
        }
        if self.overlays.notices.pending || self.overlays.notices.failed {
            return;
        }
        if !needed && self.overlays.notices.window.is_none() {
            return;
        }
        self.overlays.notices.pending = true;
        let owner = cx.entity().downgrade();
        // Run outside the owner's entity update: opening a window paints it immediately.
        window.defer(cx, move |window, cx| synchronize(owner, window, cx));
    }
}

fn synchronize(owner: WeakEntity<FarcasterApp>, window: &mut Window, cx: &mut App) {
    let Some(app) = owner.upgrade() else { return };
    let (wanted, mut popup) = {
        let app = app.read(cx);
        (
            app.notices_need_native_window()
                && window.is_window_active()
                && window.visibility() == gpui::WindowVisibility::Visible,
            app.overlays
                .notices
                .window
                .filter(|popup| cx.windows().contains(&(*popup).into())),
        )
    };
    let bounds = window.bounds();
    let viewport = window.viewport_size();
    if let Some(existing) = popup {
        let stale = existing.read(cx).map_or(true, |view| {
            matches!(view.host, NativeHost::Wayland) && view.owner_size != viewport
        });
        if !wanted || stale {
            let _ = existing.update(cx, |_, window, cx| {
                super::native_overlay::stack::close(window, cx)
            });
            popup = None;
        }
    }
    let result = if wanted {
        if let Some(existing) = popup {
            existing
                .update(cx, |view, window, cx| {
                    view.owner_bounds = bounds;
                    view.owner_size = viewport;
                    if let Err(error) =
                        view.host
                            .position(window, view.bounds(window), view.offset(window))
                    {
                        zlog::warn!("Position notification window: {error}");
                    }
                    cx.notify();
                })
                .map(|_| existing)
                .map_err(|error| error.to_string())
        } else {
            open(owner, window, cx)
        }
        .map(Some)
    } else {
        Ok(None)
    };
    app.update(cx, |app, cx| {
        app.overlays.notices.pending = false;
        match result {
            Ok(next) => {
                let changed = app.overlays.notices.window != next;
                app.overlays.notices.window = next;
                if changed {
                    cx.notify();
                }
            }
            Err(error) => {
                app.overlays.notices.window = None;
                app.overlays.notices.failed = true;
                zlog::warn!("Open notification window: {error}");
                cx.notify();
            }
        }
    });
}

fn open(
    owner: WeakEntity<FarcasterApp>,
    window: &Window,
    cx: &mut App,
) -> Result<WindowHandle<NoticeWindow>, String> {
    let host = NativeHost::capture(window)?;
    let owner_window = window.window_handle();
    let owner_bounds = window.bounds();
    let owner_size = window.viewport_size();
    let bounds = popup_bounds(owner_bounds, owner_size, px(64.0));
    let anchor = Bounds::new(
        point(owner_size.width - window_inset(), window_inset()),
        size(px(0.0), px(0.0)),
    );
    let options = WindowOptions {
        kind: host.kind(owner_window, anchor),
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
    // show=false lets us configure AppKit's focus behavior before the panel becomes visible.
    let handle = cx
        .open_window(options, move |_, cx| {
            cx.new(|_| NoticeWindow {
                owner,
                owner_window,
                owner_bounds,
                owner_size,
                host,
            })
        })
        .map_err(|error| error.to_string())?;
    let configured = handle
        .update(cx, |view, window, _| {
            view.host.present(
                window,
                view.bounds(window),
                view.offset(window),
                OverlayInput::Passive,
            )
        })
        .map_err(|error| error.to_string())?;
    if handle.read(cx).is_ok_and(|view| view.host.is_wayland()) {
        super::native_overlay::stack::register(owner_window, handle.into(), cx);
    }
    if let Err(error) = configured {
        let _ = handle.update(cx, |_, window, cx| {
            super::native_overlay::stack::close(window, cx)
        });
        return Err(error);
    }
    Ok(handle)
}

fn popup_bounds(owner: Bounds<Pixels>, viewport: Size<Pixels>, height: Pixels) -> Bounds<Pixels> {
    let width = theme()
        .size(360.0)
        .min((viewport.width - window_inset() * 2.0).max(px(1.0)));
    let height = height.min((viewport.height - window_inset() * 2.0).max(px(1.0)));
    Bounds::new(
        owner.origin + point(viewport.width - window_inset() - width, window_inset()),
        size(width, height),
    )
}

impl NoticeWindow {
    fn offset(&self, window: &Window) -> gpui::Point<Pixels> {
        self.bounds(window).origin - self.owner_bounds.origin
    }

    fn bounds(&self, window: &Window) -> Bounds<Pixels> {
        popup_bounds(
            self.owner_bounds,
            self.owner_size,
            window.viewport_size().height,
        )
    }
}

impl Render for NoticeWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(owner) = self.owner.upgrade() else {
            return div().into_any_element();
        };
        let content = owner
            .read(cx)
            .render_notice_stack(self.owner.clone(), Some(self.owner_window));
        let host = self.host.clone();
        let owner_bounds = self.owner_bounds;
        let owner_size = self.owner_size;
        div()
            .font(ui_font())
            .text_color(theme().colors.text)
            .text_size(theme().type_scale.body)
            .w_full()
            .child(
                content
                    .id("native-floating-notices")
                    .w_full()
                    .p(theme().space.xs)
                    .max_h((owner_size.height - window_inset() * 2.0).max(px(1.0)))
                    .overflow_y_scroll()
                    .on_prepaint(move |bounds, window, cx| {
                        let height = px(bounds.size.height.as_f32().ceil().max(1.0));
                        let desired = popup_bounds(owner_bounds, owner_size, height);
                        if (window.viewport_size().height - desired.size.height).abs() < px(1.0)
                            && window.viewport_size().width == desired.size.width
                        {
                            return;
                        }
                        let host = host.clone();
                        window.defer(cx, move |window, _| {
                            window.resize(desired.size);
                            if let Err(error) =
                                host.position(window, desired, desired.origin - owner_bounds.origin)
                            {
                                zlog::warn!("Resize notification window: {error}");
                            }
                        });
                    }),
            )
            .into_any_element()
    }
}

#[cfg(test)]
#[path = "notice_window_tests.rs"]
mod tests;
