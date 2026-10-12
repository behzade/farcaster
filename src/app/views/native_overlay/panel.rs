use std::marker::PhantomData;

use gpui::{AnyWindowHandle, App, Context, Entity, Render, Window, WindowHandle};

// Wayland panels mount a Base Root so the component library can register the
// focused text input. Keep callers independent of that platform detail.
pub(in crate::app) enum Panel<V> {
    Direct(WindowHandle<V>),
    Root(WindowHandle<gpui_base::Root>, PhantomData<fn() -> V>),
}

impl<V> Copy for Panel<V> {}
impl<V> Clone for Panel<V> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<V: 'static> PartialEq for Panel<V> {
    fn eq(&self, other: &Self) -> bool {
        AnyWindowHandle::from(*self) == AnyWindowHandle::from(*other)
    }
}
impl<V: 'static> Eq for Panel<V> {}

impl<V: 'static> std::fmt::Debug for Panel<V> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        AnyWindowHandle::from(*self).fmt(formatter)
    }
}

impl<V: 'static> From<Panel<V>> for AnyWindowHandle {
    fn from(panel: Panel<V>) -> Self {
        match panel {
            Panel::Direct(window) => window.into(),
            Panel::Root(window, _) => window.into(),
        }
    }
}

impl<V: Render + 'static> Panel<V> {
    pub(super) fn root(window: WindowHandle<gpui_base::Root>) -> Self {
        Self::Root(window, PhantomData)
    }

    pub(super) fn from_window(window: &Window, cx: &App) -> Option<Entity<V>> {
        window.root::<V>().flatten().or_else(|| {
            window
                .root::<gpui_base::Root>()??
                .read(cx)
                .view()
                .clone()
                .downcast::<V>()
                .ok()
        })
    }

    pub(in crate::app) fn read<'a>(&self, cx: &'a App) -> gpui::Result<&'a V> {
        match self {
            Self::Direct(window) => window.read(cx),
            Self::Root(window, _) => Ok(window
                .read(cx)?
                .view()
                .clone()
                .downcast::<V>()
                .expect("panel content")
                .read(cx)),
        }
    }

    pub(in crate::app) fn update<R>(
        &self,
        cx: &mut App,
        update: impl FnOnce(&mut V, &mut Window, &mut Context<V>) -> R,
    ) -> gpui::Result<R> {
        match self {
            Self::Direct(window) => window.update(cx, update),
            Self::Root(window, _) => window.update(cx, |root, window, cx| {
                let view = root.view().clone().downcast::<V>().expect("panel content");
                view.update(cx, |view, cx| update(view, window, cx))
            }),
        }
    }
}
