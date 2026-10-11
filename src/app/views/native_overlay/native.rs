use gpui::{Bounds, Pixels, Window, WindowKind};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

#[derive(Clone, Copy)]
pub(in crate::app::views) enum OverlayInput {
    Passive,
    Modal,
}

#[derive(Clone)]
pub(in crate::app::views) enum NativeHost {
    #[cfg(target_os = "macos")]
    Mac(macos::Host),
    #[cfg(target_os = "linux")]
    X11(std::rc::Rc<x11rb::rust_connection::RustConnection>),
    Wayland,
    #[cfg(test)]
    Test,
}

impl NativeHost {
    pub(in crate::app::views) fn capture(window: &Window) -> Result<Self, String> {
        let handle = match HasWindowHandle::window_handle(window) {
            Ok(handle) => handle.as_raw(),
            #[cfg(test)]
            Err(_) => return Ok(Self::Test),
            #[cfg(not(test))]
            Err(error) => return Err(error.to_string()),
        };
        match handle {
            #[cfg(target_os = "macos")]
            RawWindowHandle::AppKit(_) => macos::Host::capture(window).map(Self::Mac),
            #[cfg(target_os = "linux")]
            RawWindowHandle::Xcb(_) | RawWindowHandle::Xlib(_) => x11rb::connect(None)
                .map(|(connection, _)| Self::X11(std::rc::Rc::new(connection)))
                .map_err(|error| error.to_string()),
            RawWindowHandle::Wayland(_) => Ok(Self::Wayland),
            _ => Err("native notification windows are unavailable on this platform".into()),
        }
    }

    pub(in crate::app::views) fn kind(
        &self,
        parent: gpui::AnyWindowHandle,
        anchor: Bounds<Pixels>,
    ) -> WindowKind {
        if matches!(self, Self::Wayland) {
            WindowKind::AnchoredPopup(gpui::popup::PopupOptions {
                parent,
                anchor_rect: anchor,
                anchor: gpui::popup::PopupAnchor::TopRight,
                gravity: gpui::popup::PopupGravity::BottomLeft,
                constraint_adjustment: gpui::popup::PopupConstraintAdjustment::SLIDE_X
                    | gpui::popup::PopupConstraintAdjustment::SLIDE_Y,
                offset: gpui::point(gpui::px(0.0), gpui::px(0.0)),
                grab: false,
            })
        } else {
            WindowKind::PopUp
        }
    }

    pub(in crate::app::views) fn present(
        &self,
        window: &Window,
        bounds: Bounds<Pixels>,
        offset: gpui::Point<Pixels>,
        input: OverlayInput,
    ) -> Result<(), String> {
        #[cfg(target_os = "macos")]
        if let Self::Mac(host) = self {
            return host.present(window, bounds, offset, input);
        }
        // GPUI maps X11 and Wayland windows when opening them, even with show=false.
        let _ = input;
        self.position(window, bounds, offset)
    }

    pub(in crate::app::views) fn position(
        &self,
        window: &Window,
        bounds: Bounds<Pixels>,
        offset: gpui::Point<Pixels>,
    ) -> Result<(), String> {
        let _ = offset;
        match self {
            #[cfg(target_os = "macos")]
            Self::Mac(host) => host.position(window, bounds, offset),
            #[cfg(target_os = "linux")]
            Self::X11(connection) => {
                use x11rb::{
                    connection::Connection as _,
                    protocol::xproto::{ConfigureWindowAux, ConnectionExt as _},
                };
                let handle =
                    HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?;
                let id = match handle.as_raw() {
                    RawWindowHandle::Xcb(handle) => handle.window.get(),
                    RawWindowHandle::Xlib(handle) => handle.window as u32,
                    _ => return Err("notification window is not an X11 window".into()),
                };
                let scale = window.scale_factor();
                connection
                    .configure_window(
                        id,
                        &ConfigureWindowAux::new()
                            .x((bounds.origin.x.as_f32() * scale).round() as i32)
                            .y((bounds.origin.y.as_f32() * scale).round() as i32),
                    )
                    .map_err(|error| error.to_string())?
                    .check()
                    .map_err(|error| error.to_string())?;
                connection.flush().map_err(|error| error.to_string())
            }
            _ => Ok(()),
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use gpui::{Bounds, Pixels, Window};
    use objc2::{MainThreadMarker, rc::Retained};
    use objc2_app_kit::{
        NSApplication, NSPanel, NSView, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    #[derive(Clone)]
    pub(in crate::app::views) struct Host {
        window: Retained<NSWindow>,
        view: Retained<NSView>,
    }

    fn find_view(
        view: Retained<NSView>,
        pointer: *mut std::ffi::c_void,
    ) -> Option<Retained<NSView>> {
        if Retained::as_ptr(&view)
            .cast_mut()
            .cast::<std::ffi::c_void>()
            == pointer
        {
            return Some(view);
        }
        view.subviews()
            .iter()
            .find_map(|child| find_view(child, pointer))
    }

    impl Host {
        pub(super) fn capture(window: &Window) -> Result<Self, String> {
            let main =
                MainThreadMarker::new().ok_or("notification window requires the main thread")?;
            let handle =
                HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?;
            let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
                return Err("notification owner is not an AppKit window".into());
            };
            // Match a retained AppKit view to GPUI's opaque handle without dereferencing it.
            NSApplication::sharedApplication(main)
                .windows()
                .iter()
                .find_map(|native| {
                    find_view(native.contentView()?, handle.ns_view.as_ptr()).map(|view| Self {
                        window: native,
                        view,
                    })
                })
                .ok_or_else(|| "could not find the native notification window".into())
        }

        pub(super) fn position(
            &self,
            popup: &Window,
            bounds: Bounds<Pixels>,
            offset: gpui::Point<Pixels>,
        ) -> Result<(), String> {
            self.position_window(&Self::capture(popup)?.window, bounds, offset);
            Ok(())
        }

        pub(super) fn present(
            &self,
            popup: &Window,
            bounds: Bounds<Pixels>,
            offset: gpui::Point<Pixels>,
            input: super::OverlayInput,
        ) -> Result<(), String> {
            let popup = Self::capture(popup)?;
            let panel = popup
                .window
                .downcast_ref::<NSPanel>()
                .ok_or("notification window is not a panel")?;
            panel.setStyleMask(NSWindowStyleMask::NonactivatingPanel);
            panel.setBecomesKeyOnlyIfNeeded(matches!(input, super::OverlayInput::Passive));
            panel.setHidesOnDeactivate(true);
            panel.setHasShadow(false);
            self.position_window(&popup.window, bounds, offset);
            match input {
                super::OverlayInput::Passive => popup.window.orderFront(None),
                super::OverlayInput::Modal => {
                    panel.setCollectionBehavior(
                        NSWindowCollectionBehavior::MoveToActiveSpace
                            | NSWindowCollectionBehavior::FullScreenAuxiliary,
                    );
                    popup.window.makeKeyAndOrderFront(None);
                }
            }
            Ok(())
        }

        fn position_window(
            &self,
            popup: &NSWindow,
            bounds: Bounds<Pixels>,
            offset: gpui::Point<Pixels>,
        ) {
            let rect = NSRect::new(
                NSPoint::new(
                    offset.x.as_f32() as f64,
                    self.view.bounds().size.height
                        - bounds.size.height.as_f32() as f64
                        - offset.y.as_f32() as f64,
                ),
                NSSize::new(
                    bounds.size.width.as_f32() as f64,
                    bounds.size.height.as_f32() as f64,
                ),
            );
            let rect = self
                .window
                .convertRectToScreen(self.view.convertRect_toView(rect, None));
            popup.setFrame_display(rect, true);
        }
    }
}
