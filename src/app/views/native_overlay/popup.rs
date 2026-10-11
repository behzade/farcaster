use std::{collections::HashMap, rc::Rc};

use gpui::{
    AnyElement, AnyWindowHandle, App, AppContext as _, Bounds, Context, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, Styled as _,
    Subscription, WeakEntity, Window, WindowHandle, div, point, px,
};

use super::{
    FarcasterApp,
    native::{NativeHost, OverlayInput},
};
use crate::app::ui::{
    primitives::AppTooltips,
    theme::{theme, ui_font},
};

pub(in crate::app) type Builder = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;
pub(in crate::app) type Dismiss = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Default)]
pub(in crate::app) struct PopupState {
    pub(in crate::app) window: Option<WindowHandle<PopupWindow>>,
    pub(in crate::app) failed: bool,
    owner_window: Option<AnyWindowHandle>,
    subscriptions: Vec<Subscription>,
}

pub(in crate::app) struct PopupWindow {
    pub(in crate::app) owner_window: AnyWindowHandle,
    pub(in crate::app) tooltips: Entity<AppTooltips>,
    host: NativeHost,
    builder: Builder,
    focus: Option<FocusHandle>,
    input: OverlayInput,
    owner: Option<WeakEntity<FarcasterApp>>,
    _escape: Option<Subscription>,
    _tooltip_watch: Subscription,
}

#[derive(Default)]
struct Owners(HashMap<gpui::WindowId, WeakEntity<FarcasterApp>>);
impl gpui::Global for Owners {}

pub(super) fn register_owner(window: &Window, owner: WeakEntity<FarcasterApp>, cx: &mut App) {
    if !cx.has_global::<Owners>() {
        cx.set_global(Owners::default());
    }
    cx.global_mut::<Owners>()
        .0
        .insert(window.window_handle().window_id(), owner);
}

pub(super) fn release_owner(window: gpui::WindowId, cx: &mut App) {
    if cx.has_global::<Owners>() {
        cx.global_mut::<Owners>().0.remove(&window);
    }
}

pub(in crate::app) fn app_owner(window: &Window, cx: &App) -> Option<Entity<FarcasterApp>> {
    cx.try_global::<Owners>()?
        .0
        .get(&window.window_handle().window_id())?
        .upgrade()
}

pub(super) fn owner_active(window: &Window, cx: &mut App) -> bool {
    let owner = window.window_handle();
    window.is_window_active()
        || cx.windows().into_iter().any(|handle| {
            handle.downcast::<PopupWindow>().is_some_and(|popup| {
                popup
                    .update(cx, |view, window, _| {
                        view.owner_window == owner && window.is_window_active()
                    })
                    .unwrap_or(false)
            })
        })
}

pub(in crate::app) fn uses_native_popups(window: &Window, cx: &App) -> bool {
    app_owner(window, cx).is_some_and(|owner| {
        let app = owner.read(cx);
        app.native_overlays_available()
            && (!app.modal_rendered_in_native_window() || app.native_request_is_passive())
            && matches!(
                app.workspace.surface,
                crate::app::AppSurface::Editor | crate::app::AppSurface::Terminal
            )
    })
}

// The source keeps this keyed state alive. Removing a trigger closes its panel,
// even if its content still holds menu or popover entities.
pub(in crate::app) fn prepare(
    state: Entity<PopupState>,
    visible: bool,
    builder: Builder,
    input: OverlayInput,
    dismiss: Option<Dismiss>,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    if !visible && state.read(cx).window.is_none() {
        return true;
    }
    state.update(cx, |state, cx| {
        if state.subscriptions.is_empty() {
            state.owner_window = Some(window.window_handle());
            state.subscriptions = vec![
                cx.observe_window_activation(window, |_, window, _| window.refresh()),
                cx.observe_window_bounds(window, |_, window, _| window.refresh()),
                cx.observe_window_visibility(window, |_, _, window, _| window.refresh()),
                cx.on_release(|state, cx| {
                    if let Some(popup) = state.window.take() {
                        let active = popup
                            .update(cx, |_, window, _| {
                                let active = window.is_window_active();
                                window.remove_window();
                                active
                            })
                            .unwrap_or(false);
                        if active && let Some(owner) = state.owner_window {
                            let _ = owner.update(cx, |_, window, _| {
                                if window.visibility() == gpui::WindowVisibility::Visible {
                                    window.activate_window();
                                }
                            });
                        }
                    }
                }),
            ];
        }
        if !visible {
            state.failed = false;
        }
    });
    if state.read(cx).failed {
        return false;
    }
    let focus = matches!(input, OverlayInput::Modal)
        .then(|| window.focused(cx))
        .flatten();
    window.defer(cx, move |window, cx| {
        let existing = state.read(cx).window;
        let visible = visible
            && window.visibility() == gpui::WindowVisibility::Visible
            && (window.is_window_active()
                || (matches!(input, OverlayInput::Modal) && existing.is_some()));
        let bounds = Bounds::new(window.bounds().origin, window.viewport_size());
        let result = if !visible {
            if let Some(popup) = existing {
                let active = popup
                    .update(cx, |_, window, _| {
                        let active = window.is_window_active();
                        window.remove_window();
                        active
                    })
                    .unwrap_or(false);
                if active && window.visibility() == gpui::WindowVisibility::Visible {
                    window.activate_window();
                }
            }
            Ok(None)
        } else if let Some(popup) = existing {
            popup
                .update(cx, |view, window, cx| {
                    view.builder = builder;
                    if window.bounds().size != bounds.size {
                        window.resize(bounds.size);
                    }
                    view.host
                        .position(window, bounds, point(px(0.0), px(0.0)))?;
                    cx.notify();
                    Ok(Some(popup))
                })
                .map_err(|e| e.to_string())
                .and_then(|r| r)
        } else {
            open(window, bounds, builder, focus, input, dismiss, cx).map(Some)
        };
        state.update(cx, |state, cx| match result {
            Ok(popup) => state.window = popup,
            Err(error) => {
                if let Some(popup) = existing {
                    let _ = popup.update(cx, |_, window, _| window.remove_window());
                }
                state.window = None;
                state.failed = true;
                zlog::warn!("Native popup: {error}");
                if let Some(app) = app_owner(window, cx) {
                    app.update(cx, |app, cx| {
                        app.overlays.modal.failed = true;
                        cx.notify();
                    });
                }
                window.refresh();
            }
        });
    });
    true
}

fn open(
    window: &Window,
    bounds: Bounds<Pixels>,
    builder: Builder,
    focus: Option<FocusHandle>,
    input: OverlayInput,
    dismiss: Option<Dismiss>,
    cx: &mut App,
) -> Result<WindowHandle<PopupWindow>, String> {
    let owner_window = window.window_handle();
    let owner = app_owner(window, cx).map(|owner| owner.downgrade());
    NativeHost::open_panel(
        window,
        bounds,
        input,
        point(px(0.0), px(0.0)),
        move |host, window, cx| {
            cx.new(|cx| {
                let tooltips = cx.new(|_| AppTooltips::default());
                let watch = cx.observe(&tooltips, |_, _, cx| cx.notify());
                let escape = dismiss.map(|dismiss| {
                    let id = window.window_handle().window_id();
                    cx.intercept_keystrokes(move |event, window, cx| {
                        if window.window_handle().window_id() == id
                            && event.keystroke.key == "escape"
                            && !event.keystroke.modifiers.modified()
                        {
                            dismiss(window, cx);
                            cx.stop_propagation();
                        }
                    })
                });
                PopupWindow {
                    owner_window,
                    owner,
                    host,
                    builder,
                    focus,
                    input,
                    tooltips,
                    _escape: escape,
                    _tooltip_watch: watch,
                }
            })
        },
        cx,
    )
}

impl Render for PopupWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(focus) = self.focus.take() {
            focus.focus(window, cx);
        }
        let root = div()
            .relative()
            .size_full()
            .font(ui_font())
            .text_size(theme().type_scale.body)
            .text_color(theme().colors.text)
            .key_context(crate::app::APP_INPUT_CONTEXT)
            .on_key_down(|event, window, cx| {
                crate::app::ui::focus::traverse_tab(event, None, window, cx);
            });
        let root = if let Some(owner) = self.owner.as_ref().and_then(WeakEntity::upgrade) {
            owner.update(cx, |_, cx| {
                super::super::root::bind_overlay_actions(root, cx)
            })
        } else {
            root
        };
        let root = root
            .child((self.builder)(window, cx))
            .child(self.tooltips.clone());
        if matches!(self.input, OverlayInput::Modal)
            && let Some(owner) = self.owner.as_ref().and_then(WeakEntity::upgrade)
        {
            return root.children(owner.update(cx, |app, cx| {
                app.has_floating_notices().then(|| {
                    app.render_floating_notices(cx.entity().downgrade(), Some(self.owner_window))
                })
            }));
        }
        root
    }
}
