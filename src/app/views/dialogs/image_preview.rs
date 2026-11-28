use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AvailableSpace, Bounds, Context, CursorStyle, DispatchPhase,
    InteractiveElement as _, IntoElement as _, MouseButton, MouseMoveEvent, MouseUpEvent,
    ObjectFit, ParentElement as _, Pixels, Point, Render, RenderImage, Size,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, WeakEntity, Window, canvas,
    div, img, point, px, relative, size,
};
use std::sync::Arc;

use crate::app::FarcasterApp;
use crate::{
    app::ui::assets::AppIcon,
    app::ui::primitives::{
        AppIconSize, ButtonTone, button, icon_button, icon_button_with_size, modal,
    },
    app::ui::theme::theme,
};

pub(in crate::app::views) fn render(
    app: &FarcasterApp,
    entity: WeakEntity<FarcasterApp>,
) -> Option<AnyElement> {
    let preview = app.overlays.image_preview.as_ref()?.clone();
    let close = entity.clone();
    Some(
        modal(
            "image-preview",
            format!("Image {} of {}", preview.index + 1, preview.total),
            &app.overlays.image_preview_focus,
            crate::app::OVERLAY_KEY_CONTEXT,
            move |window, cx| {
                let _ = close.update(cx, |this, cx| this.close_image_preview(window, cx));
            },
            |surface| {
                let close = entity.clone();
                let dismiss = entity.clone();
                let viewport = preview.viewport.clone();
                surface
                    .debug_selector(|| "image-preview-surface".into())
                    .relative()
                    .w(relative(0.8))
                    .h(relative(0.8))
                    .max_w(theme().size(1120.0))
                    .max_h(theme().size(800.0))
                    .rounded_none()
                    .shadow(Vec::new())
                    .bg(gpui::transparent_black())
                    .overflow_hidden()
                    .on_click(move |event, window, cx| {
                        if event.is_keyboard() {
                            return;
                        }
                        let view = viewport.read(cx);
                        if !view.pressed_image && !view.image_bounds().contains(&event.position()) {
                            let _ = dismiss.update(cx, |this, cx| {
                                this.close_image_preview(window, cx);
                            });
                        }
                    })
                    .child(preview.viewport.clone())
                    .child(
                        div()
                            .id("image-preview-controls")
                            .debug_selector(|| "image-preview-controls".into())
                            .occlude()
                            .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .absolute()
                            .top(theme().space.sm)
                            .right(theme().space.sm)
                            .flex()
                            .gap(theme().space.xs)
                            .child(icon_button(
                                "open-image-externally",
                                AppIcon::ArrowSquareOut,
                                "Open image externally",
                                ButtonTone::Quiet,
                                {
                                    let entity = entity.clone();
                                    let image = preview.image.clone();
                                    move |window, cx| {
                                        let _ = entity.update(cx, |this, cx| {
                                            this.open_image_externally(image.clone(), window, cx)
                                        });
                                    }
                                },
                            ))
                            .when(preview.gallery.len() > 1, |controls| {
                                controls
                                    .child(div().child(format!(
                                        "{} / {}",
                                        preview.index + 1,
                                        preview.total
                                    )))
                                    .children(
                                        [
                                            (-1isize, "Previous", AppIcon::CaretLeft),
                                            (1, "Next", AppIcon::CaretRight),
                                        ]
                                        .into_iter()
                                        .map(
                                            |(delta, label, icon)| {
                                                let entity = entity.clone();
                                                let gallery = preview.gallery.clone();
                                                let index = (preview.index as isize + delta)
                                                    .rem_euclid(gallery.len() as isize)
                                                    as usize;
                                                icon_button_with_size(
                                                    if delta < 0 {
                                                        "image-gallery-previous"
                                                    } else {
                                                        "image-gallery-next"
                                                    },
                                                    icon,
                                                    label,
                                                    AppIconSize::Control,
                                                    ButtonTone::Quiet,
                                                    move |window, cx| {
                                                        let _ = entity.update(cx, |this, cx| {
                                                            this.open_image_gallery(
                                                                gallery.clone(),
                                                                index,
                                                                window,
                                                                cx,
                                                            )
                                                        });
                                                    },
                                                )
                                                .debug_selector(move || {
                                                    if delta < 0 {
                                                        "image-gallery-previous".into()
                                                    } else {
                                                        "image-gallery-next".into()
                                                    }
                                                })
                                            },
                                        ),
                                    )
                            })
                            .child(icon_button(
                                "close-image-preview",
                                AppIcon::X,
                                "Close image preview",
                                ButtonTone::Quiet,
                                move |window, cx| {
                                    let _ = close.update(cx, |this, cx| {
                                        this.close_image_preview(window, cx)
                                    });
                                },
                            )),
                    )
            },
        )
        .into_any_element(),
    )
}

pub(in crate::app) struct ImageViewport {
    image: Arc<RenderImage>,
    zoom: f32,
    pan: Point<Pixels>,
    drag: Option<Point<Pixels>>,
    pressed_image: bool,
    bounds: Bounds<Pixels>,
}

impl ImageViewport {
    pub(in crate::app) fn new(image: Arc<RenderImage>) -> Self {
        Self {
            image,
            zoom: 1.0,
            pan: Point::default(),
            drag: None,
            pressed_image: false,
            bounds: Bounds::default(),
        }
    }

    fn image_size(&self) -> Size<Pixels> {
        let native = self.image.size(0);
        let native = size(
            px(u32::from(native.width) as f32),
            px(u32::from(native.height) as f32),
        );
        let fit = (self.bounds.size.width / native.width)
            .min(self.bounds.size.height / native.height)
            .min(1.0);
        native.map(|dimension| dimension * (fit * self.zoom))
    }

    fn pan_limit(&self) -> Point<Pixels> {
        let image = self.image_size();
        point(
            ((image.width - self.bounds.size.width) / 2.0).max(px(0.0)),
            ((image.height - self.bounds.size.height) / 2.0).max(px(0.0)),
        )
    }

    fn clamp_pan(&mut self) {
        let limit = self.pan_limit();
        self.pan.x = self.pan.x.clamp(-limit.x, limit.x);
        self.pan.y = self.pan.y.clamp(-limit.y, limit.y);
    }

    fn image_bounds(&self) -> Bounds<Pixels> {
        let image = self.image_size();
        Bounds::new(
            self.bounds.center() + self.pan - point(image.width / 2.0, image.height / 2.0),
            image,
        )
    }

    fn zoom_at(&mut self, pointer: Point<Pixels>, delta: f32) {
        let native = self.image.size(0);
        let fit = (self.bounds.size.width / px(u32::from(native.width) as f32))
            .min(self.bounds.size.height / px(u32::from(native.height) as f32))
            .min(1.0);
        let zoom = (self.zoom * (delta * 0.005).exp())
            .clamp(1.0, 32.0_f32.max(1.0 / fit.max(f32::EPSILON)));
        let pointer = pointer - self.bounds.center();
        self.pan = pointer - (pointer - self.pan) * (zoom / self.zoom);
        self.zoom = zoom;
        self.drag = None;
        self.clamp_pan();
    }

    fn move_pointer(&mut self, event: &MouseMoveEvent) {
        if !event.dragging() {
            self.drag = None;
            return;
        }
        if let Some(previous) = self.drag {
            self.drag = Some(event.position);
            self.pan += event.position - previous;
            self.clamp_pan();
        }
    }
}

impl Render for ImageViewport {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let layout = cx.entity();
        let pointer = cx.entity().downgrade();
        let release = pointer.clone();
        let limit = self.pan_limit();
        let can_pan = limit.x > px(0.0) || limit.y > px(0.0);
        div()
            .id("image-preview-viewport")
            .absolute()
            .inset_0()
            .size_full()
            .overflow_hidden()
            .cursor(if self.drag.is_some() {
                CursorStyle::ClosedHand
            } else if can_pan {
                CursorStyle::OpenHand
            } else {
                CursorStyle::Arrow
            })
            .on_scroll(cx.listener(|this, event: &gpui::ScrollEvent, _, cx| {
                cx.stop_propagation();
                let delta = event.delta.pixel_delta(px(24.0)).y;
                if delta != px(0.0) {
                    this.zoom_at(event.position, f32::from(delta));
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    this.pressed_image = this.image_bounds().contains(&event.position);
                    let limit = this.pan_limit();
                    if (limit.x > px(0.0) || limit.y > px(0.0)) && this.pressed_image {
                        cx.stop_propagation();
                        window.prevent_default();
                        this.drag = Some(event.position);
                        cx.notify();
                    }
                }),
            )
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let (mut image, bounds) = layout.update(cx, |this, _| {
                            this.bounds = bounds;
                            this.clamp_pan();
                            let bounds = this.image_bounds();
                            (
                                div()
                                    .relative()
                                    .debug_selector(|| "image-preview-image".into())
                                    .w(bounds.size.width)
                                    .h(bounds.size.height)
                                    .child(
                                        img(this.image.clone())
                                            .id("image-preview-pixels")
                                            .absolute()
                                            .inset_0()
                                            .size_full()
                                            .object_fit(ObjectFit::Contain),
                                    )
                                    .into_any_element(),
                                bounds,
                            )
                        });
                        image.layout_as_root(AvailableSpace::min_size(), window, cx);
                        image.prepaint_at(bounds.origin, window, cx);
                        image
                    },
                    move |_, mut image, window, cx| {
                        image.paint(window, cx);
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble {
                                let _ = pointer.update(cx, |this, cx| {
                                    if this.drag.is_some() {
                                        cx.stop_propagation();
                                        this.move_pointer(event);
                                        cx.notify();
                                    }
                                });
                            }
                        });
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                                let _ = release.update(cx, |this, cx| {
                                    if this.drag.take().is_some() {
                                        cx.notify();
                                    }
                                });
                            }
                        });
                    },
                )
                .size_full(),
            )
            .child(
                div()
                    .id("image-preview-zoom-controls")
                    .absolute()
                    .bottom(theme().space.sm)
                    .left(theme().space.sm)
                    .right(theme().space.sm)
                    .occlude()
                    .on_any_mouse_down(|_, _, cx| cx.stop_propagation())
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(theme().space.sm)
                    .bg(theme().colors.panel)
                    .px(theme().space.sm)
                    .child(format!(
                        "{:.0}%",
                        self.image_size().width / px(u32::from(self.image.size(0).width) as f32)
                            * 100.0
                    ))
                    .child(
                        button("image-fit", "Fit", ButtonTone::Quiet, true, {
                            let view = cx.entity();
                            move |_, cx| {
                                view.update(cx, |this, cx| {
                                    this.zoom = 1.0;
                                    this.pan = Point::default();
                                    this.drag = None;
                                    cx.notify();
                                })
                            }
                        })
                        .debug_selector(|| "image-fit".into()),
                    )
                    .child(
                        button("image-native-size", "100%", ButtonTone::Quiet, true, {
                            let view = cx.entity();
                            move |_, cx| {
                                view.update(cx, |this, cx| {
                                    let native = this.image.size(0);
                                    let fit = (this.bounds.size.width
                                        / px(u32::from(native.width) as f32))
                                    .min(
                                        this.bounds.size.height
                                            / px(u32::from(native.height) as f32),
                                    )
                                    .min(1.0);
                                    this.zoom = 1.0 / fit.max(f32::EPSILON);
                                    this.pan = Point::default();
                                    this.drag = None;
                                    cx.notify();
                                })
                            }
                        })
                        .debug_selector(|| "image-native-size".into()),
                    )
                    .child(
                        div()
                            .text_size(theme().type_scale.caption)
                            .child("Scroll to zoom · Drag to move"),
                    ),
            )
    }
}

#[cfg(test)]
#[path = "image_preview_tests.rs"]
mod tests;
