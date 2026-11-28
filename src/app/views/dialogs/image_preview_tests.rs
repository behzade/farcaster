use gpui::{Image, ImageFormat, ObjectFit, px, size};
use std::sync::Arc;

#[gpui::test]
fn image_preview_fits_wide_tall_and_small_images_after_resize(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::image_preview_fits_wide_tall_and_small_images_after_resize"
        ),
        cx,
        |cx, app, _, _| {
            for (width, height) in [(4096, 256), (256, 4096), (32, 32)] {
                let mut bytes = std::io::Cursor::new(Vec::new());
                image::DynamicImage::new_rgb8(width, height)
                    .write_to(&mut bytes, image::ImageFormat::Png)
                    .unwrap();
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.open_image_preview(
                            Arc::new(Image::from_bytes(ImageFormat::Png, bytes.into_inner())),
                            0,
                            1,
                            window,
                            cx,
                        );
                    });
                });
                cx.run_until_parked();
                for (window_width, window_height) in
                    [(800., 600.), (360., 240.), (1200., 800.), (2560., 1600.)]
                {
                    cx.simulate_resize(size(px(window_width), px(window_height)));
                    cx.update(|window, cx| window.draw(cx).clear(cx));
                    let surface = cx.debug_bounds("image-preview-surface").unwrap();
                    let image = cx.debug_bounds("image-preview-image").unwrap();
                    assert!(image.size.width <= px(width as f32));
                    assert!(image.size.height <= px(height as f32));
                    assert!(surface.size.width > px(0.) && surface.size.height > px(0.));
                    assert!(surface.left() >= px(window_width * 0.1));
                    assert!(surface.top() >= px(window_height * 0.1));
                    assert!(surface.right() <= px(window_width * 0.9));
                    assert!(surface.bottom() <= px(window_height * 0.9));
                    assert!(surface.size.width <= crate::app::ui::theme::theme().size(1120.));
                    assert!(surface.size.height <= crate::app::ui::theme::theme().size(800.));
                    assert!((surface.center().x - px(window_width / 2.)).abs() <= px(1.));
                    assert!((surface.center().y - px(window_height / 2.)).abs() <= px(1.));
                    let painted =
                        ObjectFit::ScaleDown.get_bounds(image, size(width.into(), height.into()));
                    assert!(
                        painted.left() >= surface.left()
                            && painted.right() <= surface.right()
                            && painted.top() >= surface.top()
                            && painted.bottom() <= surface.bottom(),
                        "{width}x{height} image must fit at {window_width}x{window_height}: surface={surface:?}, image={image:?}, painted={painted:?}"
                    );
                }
            }
        },
    );
}

#[gpui::test]
fn image_preview_zooms_at_pointer_and_drags_overflow_until_released(cx: &mut gpui::TestAppContext) {
    use gpui::{MouseButton, ScrollDelta, ScrollEvent, point};
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::image_preview_zooms_at_pointer_and_drags_overflow_until_released"
        ),
        cx,
        |cx, app, _, _| {
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(800, 600)
                .write_to(&mut bytes, image::ImageFormat::Png)
                .unwrap();
            let image = Arc::new(Image::from_bytes(ImageFormat::Png, bytes.into_inner()));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.open_image_preview(image.clone(), 0, 1, window, cx)
                })
            });
            cx.run_until_parked();
            cx.simulate_resize(size(px(800.), px(600.)));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let viewport = cx.update(|_, cx| {
                app.read(cx)
                    .overlays
                    .image_preview
                    .as_ref()
                    .unwrap()
                    .viewport
                    .clone()
            });
            let surface = cx.debug_bounds("image-preview-surface").unwrap();
            let original = cx.debug_bounds("image-preview-image").unwrap();
            let controls = cx.debug_bounds("image-preview-controls").unwrap();
            cx.simulate_mouse_down(surface.center(), MouseButton::Left, Default::default());
            cx.simulate_mouse_move(
                surface.center() + point(px(30.), px(20.)),
                MouseButton::Left,
                Default::default(),
            );
            cx.simulate_mouse_up(surface.center(), MouseButton::Left, Default::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert_eq!(
                cx.debug_bounds("image-preview-image").unwrap(),
                original,
                "a fitted image must stay centered"
            );
            let pointer = surface.center() + point(px(60.), px(40.));
            cx.simulate_event(ScrollEvent {
                position: pointer,
                delta: ScrollDelta::Pixels(point(px(0.), px(std::f32::consts::LN_2 / 0.005))),
                ..Default::default()
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let zoomed = cx.debug_bounds("image-preview-image").unwrap();
            assert_eq!(cx.debug_bounds("image-preview-controls").unwrap(), controls);
            assert!((zoomed.size.width / original.size.width - 2.).abs() < 0.01);
            for (before, after) in [
                (
                    (pointer.x - original.left()) / original.size.width,
                    (pointer.x - zoomed.left()) / zoomed.size.width,
                ),
                (
                    (pointer.y - original.top()) / original.size.height,
                    (pointer.y - zoomed.top()) / zoomed.size.height,
                ),
            ] {
                assert!(
                    (before - after).abs() < 0.001,
                    "zoom must keep the image point beneath the pointer"
                );
            }
            cx.simulate_mouse_down(surface.center(), MouseButton::Left, Default::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let outside = surface.center() + point(px(5000.), px(5000.));
            cx.simulate_mouse_move(outside, MouseButton::Left, Default::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let moved = cx.debug_bounds("image-preview-image").unwrap();
            assert!(moved.left() > zoomed.left() && moved.top() > zoomed.top());
            assert!((moved.left() - surface.left()).abs() <= px(1.));
            assert!((moved.top() - surface.top()).abs() <= px(1.));
            cx.simulate_mouse_up(outside, MouseButton::Left, Default::default());
            cx.simulate_mouse_move(surface.center(), None, Default::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert_eq!(cx.debug_bounds("image-preview-image").unwrap(), moved);
            cx.update(|_, cx| assert!(viewport.read(cx).drag.is_none()));

            cx.simulate_resize(size(px(1200.), px(800.)));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let surface = cx.debug_bounds("image-preview-surface").unwrap();
            let resized = cx.debug_bounds("image-preview-image").unwrap();
            assert!(resized.left() <= surface.left() && resized.right() >= surface.right());
            assert!(resized.top() <= surface.top() && resized.bottom() >= surface.bottom());
            cx.simulate_event(ScrollEvent {
                position: surface.center(),
                delta: ScrollDelta::Lines(point(0., 10000.)),
                ..Default::default()
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.update(|_, cx| assert_eq!(viewport.read(cx).zoom, 32.));
            cx.simulate_event(ScrollEvent {
                position: surface.center(),
                delta: ScrollDelta::Lines(point(0., -10000.)),
                ..Default::default()
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.update(|_, cx| {
                let view = viewport.read(cx);
                assert_eq!(view.zoom, 1.);
                assert_eq!(view.pan, point(px(0.), px(0.)));
            });
            cx.simulate_event(ScrollEvent {
                position: surface.center(),
                delta: ScrollDelta::Lines(point(0., 3.)),
                ..Default::default()
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.update(|_, cx| assert!(viewport.read(cx).zoom > 1.));
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.close_image_preview(window, cx);
                    app.open_image_preview(image, 0, 1, window, cx);
                })
            });
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.update(|_, cx| {
                let view = app
                    .read(cx)
                    .overlays
                    .image_preview
                    .as_ref()
                    .unwrap()
                    .viewport
                    .read(cx);
                assert_eq!(view.zoom, 1.);
                assert!(view.drag.is_none());
            });
            cx.simulate_keystrokes("escape");
            cx.update(|_, cx| assert!(app.read(cx).overlays.image_preview.is_none()));
        },
    );
}

#[gpui::test]
fn image_preview_dismisses_empty_space_but_keeps_image_gestures_and_controls(
    cx: &mut gpui::TestAppContext,
) {
    use gpui::{Bounds, MouseButton, Pixels, ScrollDelta, ScrollEvent, point};
    fn blank_point(surface: Bounds<Pixels>, image: Bounds<Pixels>) -> gpui::Point<Pixels> {
        if image.left() > surface.left() + px(4.) {
            point(surface.left() + px(2.), surface.center().y)
        } else {
            point(surface.center().x, surface.top() + px(2.))
        }
    }
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::image_preview_dismisses_empty_space_but_keeps_image_gestures_and_controls"
        ),
        cx,
        |cx, app, _, _| {
            cx.simulate_resize(size(px(800.), px(600.)));
            for (width, height) in [(4096, 256), (256, 4096), (32, 32)] {
                let mut bytes = std::io::Cursor::new(Vec::new());
                image::DynamicImage::new_rgb8(width, height)
                    .write_to(&mut bytes, image::ImageFormat::Png)
                    .unwrap();
                let image = Arc::new(Image::from_bytes(ImageFormat::Png, bytes.into_inner()));
                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.open_image_preview(image.clone(), 0, 1, window, cx)
                    })
                });
                cx.run_until_parked();
                cx.update(|window, cx| window.draw(cx).clear(cx));
                let surface = cx.debug_bounds("image-preview-surface").unwrap();
                let bounds = cx.debug_bounds("image-preview-image").unwrap();
                cx.simulate_click(bounds.center(), Default::default());
                cx.update(|window, cx| window.draw(cx).clear(cx));
                cx.update(|_, cx| assert!(app.read(cx).overlays.image_preview.is_some()));
                let controls = cx.debug_bounds("image-preview-controls").unwrap();
                cx.simulate_click(controls.center(), Default::default());
                cx.update(|window, cx| window.draw(cx).clear(cx));
                cx.update(|_, cx| {
                    assert!(
                        app.read(cx).overlays.image_preview.is_some(),
                        "the space between controls must not dismiss"
                    )
                });
                let blank = blank_point(surface, bounds);
                assert!(!bounds.contains(&blank));
                cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Default::default());
                cx.simulate_mouse_move(blank, MouseButton::Left, Default::default());
                cx.simulate_mouse_up(blank, MouseButton::Left, Default::default());
                cx.update(|window, cx| window.draw(cx).clear(cx));
                cx.update(|_, cx| {
                    assert!(
                        app.read(cx).overlays.image_preview.is_some(),
                        "releasing an image gesture in empty space must not dismiss"
                    )
                });

                cx.simulate_event(ScrollEvent {
                    position: bounds.center(),
                    delta: ScrollDelta::Pixels(point(px(0.), px(std::f32::consts::LN_2 / 0.005))),
                    ..Default::default()
                });
                cx.update(|window, cx| window.draw(cx).clear(cx));
                let bounds = cx.debug_bounds("image-preview-image").unwrap();
                let blank = blank_point(surface, bounds);
                assert!(!bounds.contains(&blank));
                cx.simulate_mouse_down(surface.center(), MouseButton::Left, Default::default());
                cx.update(|window, cx| window.draw(cx).clear(cx));
                cx.simulate_mouse_move(blank, MouseButton::Left, Default::default());
                cx.update(|window, cx| window.draw(cx).clear(cx));
                cx.simulate_mouse_up(blank, MouseButton::Left, Default::default());
                cx.update(|window, cx| window.draw(cx).clear(cx));
                cx.update(|_, cx| {
                    assert!(
                        app.read(cx).overlays.image_preview.is_some(),
                        "releasing a pan in empty space must not dismiss"
                    )
                });
                cx.simulate_click(blank, Default::default());
                cx.update(|window, cx| window.draw(cx).clear(cx));
                cx.update(|_, cx| {
                    assert!(
                        app.read(cx).overlays.image_preview.is_none(),
                        "clicking empty space must dismiss"
                    )
                });

                cx.update(|window, cx| {
                    app.update(cx, |app, cx| {
                        app.open_image_preview(image, 0, 1, window, cx)
                    })
                });
                cx.run_until_parked();
                cx.update(|window, cx| window.draw(cx).clear(cx));
                cx.simulate_click(point(px(4.), px(4.)), Default::default());
                cx.update(|window, cx| window.draw(cx).clear(cx));
                cx.update(|_, cx| {
                    assert!(
                        app.read(cx).overlays.image_preview.is_none(),
                        "the outer backdrop must dismiss"
                    )
                });
            }
        },
    );
}

#[gpui::test]
fn image_gallery_navigation_and_zoom_buttons(cx: &mut gpui::TestAppContext) {
    crate::app::test_support::with_offline_app(
        concat!(
            module_path!(),
            "::image_gallery_navigation_and_zoom_buttons"
        ),
        cx,
        |cx, app, _, _| {
            let gallery: Vec<_> = [(1600, 1200), (400, 300)]
                .into_iter()
                .map(|(w, h)| {
                    let mut bytes = std::io::Cursor::new(Vec::new());
                    image::DynamicImage::new_rgb8(w, h)
                        .write_to(&mut bytes, image::ImageFormat::Png)
                        .unwrap();
                    Arc::new(Image::from_bytes(ImageFormat::Png, bytes.into_inner()))
                })
                .collect();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| app.open_image_gallery(gallery, 0, window, cx))
            });
            cx.run_until_parked();
            cx.simulate_resize(size(px(800.), px(600.)));
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let viewport = cx.update(|_, cx| {
                app.read(cx)
                    .overlays
                    .image_preview
                    .as_ref()
                    .unwrap()
                    .viewport
                    .clone()
            });
            let native = cx.debug_bounds("image-native-size").unwrap();
            cx.simulate_click(native.center(), Default::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert_eq!(
                cx.debug_bounds("image-preview-image").unwrap().size.width,
                px(1600.)
            );
            let fit = cx.debug_bounds("image-fit").unwrap();
            cx.simulate_click(fit.center(), Default::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            assert!(cx.debug_bounds("image-preview-image").unwrap().size.width < px(800.));
            cx.update(|_, cx| assert_eq!(viewport.read(cx).zoom, 1.));
            let next = cx.debug_bounds("image-gallery-next").unwrap();
            cx.simulate_click(next.center(), Default::default());
            cx.run_until_parked();
            cx.update(|window, cx| {
                assert_eq!(
                    app.read(cx).overlays.image_preview.as_ref().unwrap().index,
                    1
                );
                window.draw(cx).clear(cx);
            });
            assert_eq!(
                cx.debug_bounds("image-preview-image").unwrap().size.width,
                px(400.)
            );
        },
    );
}
