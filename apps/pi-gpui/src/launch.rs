//! Argument validation and native window setup.

use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

use crate::{
    app::{PiApp, QuitApplication},
    assets::AppAssets,
    keybindings, project_trust,
    project_trust_view::ProjectTrustView,
    state::{StateStore, WindowPlacement, WindowState},
    theme::{THEME, install_component_theme},
};
use gpui::{
    App, AppContext as _, Bounds, Context, DisplayId, Subscription, TitlebarOptions, WeakEntity,
    Window, WindowBounds, WindowOptions, point, px, size,
};

#[derive(Debug, thiserror::Error)]
pub(crate) enum LaunchError {
    #[error("resolve project {path}: {source}")]
    Resolve {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("project path is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("resolve project trust: {0}")]
    ProjectTrust(String),
    #[error("the bundled fonts could not be loaded")]
    BundledFonts,
    #[error("the native Pi window could not open")]
    NativeWindow,
}

pub(crate) fn resolve_project(path: Option<PathBuf>) -> Result<PathBuf, LaunchError> {
    let path = match path.or_else(crate::projects::most_recent) {
        Some(path) => path,
        None => std::env::current_dir().map_err(|source| LaunchError::Resolve {
            path: PathBuf::from("."),
            source,
        })?,
    };
    let resolved = path.canonicalize().map_err(|source| LaunchError::Resolve {
        path: path.clone(),
        source,
    })?;
    if !resolved.is_dir() {
        return Err(LaunchError::NotDirectory(resolved));
    }
    Ok(resolved)
}

pub(crate) fn run(project: PathBuf) -> Result<(), LaunchError> {
    const FONT_FAILURE: u8 = 1;
    const WINDOW_FAILURE: u8 = 2;

    let startup_trust =
        project_trust::startup_trust(&project).map_err(LaunchError::ProjectTrust)?;
    let failure = Arc::new(AtomicU8::new(0));
    let failure_in_app = failure.clone();
    gpui_platform::application()
        .with_assets(AppAssets)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            if AppAssets.load_fonts(cx).is_err() {
                failure_in_app.store(FONT_FAILURE, Ordering::Release);
                cx.quit();
                return;
            }
            install_component_theme(cx);
            let notification_app: Rc<RefCell<Option<WeakEntity<PiApp>>>> =
                Rc::new(RefCell::new(None));
            let quit_app = notification_app.clone();
            cx.on_action(move |_: &QuitApplication, cx| {
                let Some(app) = quit_app.borrow().clone() else {
                    cx.quit();
                    return;
                };
                if app
                    .update_in(cx, |app, window, cx| {
                        app.request_application_quit(window, cx)
                    })
                    .is_err()
                {
                    cx.quit();
                }
            });
            let response_app = notification_app.clone();
            cx.on_system_notification_response(move |response, cx| {
                cx.activate(true);
                let app = response_app.borrow().clone();
                if let Some(app) = app {
                    let _ = app.update_in(cx, |app, window, cx| {
                        app.activate_system_notification(&response.tag, window, cx);
                        window.activate_window();
                    });
                } else if let Some(window) = cx
                    .active_window()
                    .or_else(|| cx.windows().into_iter().next())
                {
                    let _ = cx.update_window(window, |_, window, _| window.activate_window());
                }
            });
            cx.bind_keys(
                keybindings::registry()
                    .into_iter()
                    .map(|entry| entry.binding),
            );
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let (window_bounds, display_id) = restored_window(cx).unwrap_or_else(|| {
                (
                    WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(THEME.layout.window_width, THEME.layout.window_height),
                        cx,
                    )),
                    None,
                )
            });
            let result = cx.open_window(
                WindowOptions {
                    window_bounds: Some(window_bounds),
                    display_id,
                    titlebar: Some(TitlebarOptions {
                        title: Some("Pi".into()),
                        ..TitlebarOptions::default()
                    }),
                    app_id: Some("local.pi.gpui".into()),
                    ..WindowOptions::default()
                },
                move |window, cx| {
                    let launch = cx.new(|cx| {
                        ProjectTrustView::new(
                            project.clone(),
                            startup_trust,
                            notification_app.clone(),
                            window,
                            cx,
                        )
                    });
                    cx.new(|cx| gpui_component::Root::new(launch, window, cx))
                },
            );
            if result.is_err() {
                failure_in_app.store(WINDOW_FAILURE, Ordering::Release);
                cx.quit();
                return;
            }
            cx.activate(true);
        });
    match failure.load(Ordering::Acquire) {
        FONT_FAILURE => Err(LaunchError::BundledFonts),
        WINDOW_FAILURE => Err(LaunchError::NativeWindow),
        _ => Ok(()),
    }
}

pub(crate) fn observe_window_placement<T: 'static>(
    window: &mut Window,
    cx: &mut Context<T>,
) -> Subscription {
    let pending = Rc::new(RefCell::new(None));
    cx.observe_window_bounds(window, move |_, window, cx| {
        let placement = capture_window_placement(window, cx);
        let timer = cx.background_executor().timer(Duration::from_millis(200));
        let task = cx.background_spawn(async move {
            timer.await;
            let _ = StateStore::open().and_then(|store| store.save_window_placement(&placement));
        });
        *pending.borrow_mut() = Some(task);
    })
}

fn restored_window(cx: &App) -> Option<(WindowBounds, Option<DisplayId>)> {
    let placement = StateStore::open().ok()?.load_window_placement().ok()??;
    restore_window_placement(&placement, cx)
}

fn restore_window_placement(
    placement: &WindowPlacement,
    cx: &App,
) -> Option<(WindowBounds, Option<DisplayId>)> {
    let displays = cx
        .displays()
        .into_iter()
        .map(|display| DisplayPlacement {
            id: display.id(),
            uuid: display.uuid().ok().map(|uuid| uuid.to_string()),
            bounds: display.bounds(),
            visible_bounds: display.visible_bounds(),
        })
        .collect::<Vec<_>>();
    restore_window_placement_for_displays(placement, &displays)
}

#[derive(Clone, Debug)]
struct DisplayPlacement {
    id: DisplayId,
    uuid: Option<String>,
    bounds: Bounds<gpui::Pixels>,
    visible_bounds: Bounds<gpui::Pixels>,
}

fn restore_window_placement_for_displays(
    placement: &WindowPlacement,
    displays: &[DisplayPlacement],
) -> Option<(WindowBounds, Option<DisplayId>)> {
    let [x, y, width, height] = placement.bounds;
    if ![x, y, width, height].into_iter().all(f32::is_finite) || width <= 0.0 || height <= 0.0 {
        return None;
    }

    let matched_display = placement.display_uuid.as_ref().and_then(|stored_uuid| {
        displays
            .iter()
            .find(|display| display.uuid.as_ref() == Some(stored_uuid))
    });
    let translated_origin = matched_display.map_or([x, y], |display| {
        [
            x + f32::from(display.bounds.origin.x) - placement.display_origin[0],
            y + f32::from(display.bounds.origin.y) - placement.display_origin[1],
        ]
    });
    let candidate_bounds = Bounds::new(
        point(px(translated_origin[0]), px(translated_origin[1])),
        size(px(width), px(height)),
    );
    let display = matched_display.or_else(|| {
        displays
            .iter()
            .find(|display| candidate_bounds.intersects(&display.visible_bounds))
    })?;
    let visible = display.visible_bounds;
    let left = f32::from(visible.left());
    let top = f32::from(visible.top());
    let max_x = (f32::from(visible.right()) - width).max(left);
    let max_y = (f32::from(visible.bottom()) - height).max(top);
    let bounds = Bounds::new(
        point(
            px(translated_origin[0].clamp(left, max_x)),
            px(translated_origin[1].clamp(top, max_y)),
        ),
        size(px(width), px(height)),
    );
    let bounds = match placement.state {
        WindowState::Windowed => WindowBounds::Windowed(bounds),
        WindowState::Maximized => WindowBounds::Maximized(bounds),
        WindowState::Fullscreen => WindowBounds::Fullscreen(bounds),
    };
    Some((bounds, Some(display.id)))
}

fn capture_window_placement(window: &Window, cx: &App) -> WindowPlacement {
    let window_bounds = window.window_bounds();
    let bounds = window_bounds.get_bounds();
    let display = window.display(cx);
    let display_bounds = display.as_ref().map(|display| display.bounds());
    WindowPlacement {
        bounds: [
            f32::from(bounds.origin.x),
            f32::from(bounds.origin.y),
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
        ],
        display_uuid: display
            .as_ref()
            .and_then(|display| display.uuid().ok())
            .map(|uuid| uuid.to_string()),
        display_origin: display_bounds.map_or([0.0, 0.0], |bounds| {
            [f32::from(bounds.origin.x), f32::from(bounds.origin.y)]
        }),
        state: match window_bounds {
            WindowBounds::Windowed(_) => WindowState::Windowed,
            WindowBounds::Maximized(_) => WindowState::Maximized,
            WindowBounds::Fullscreen(_) => WindowState::Fullscreen,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn project_resolution_requires_a_directory() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        assert_eq!(
            resolve_project(Some(directory.path().to_path_buf()))?,
            directory.path().canonicalize()?
        );
        let file = directory.path().join("file");
        std::fs::write(&file, "x")?;
        assert!(matches!(
            resolve_project(Some(file)),
            Err(LaunchError::NotDirectory(_))
        ));
        Ok(())
    }

    #[test]
    fn restored_window_follows_its_display_when_the_display_origin_changes() {
        let placement = WindowPlacement {
            bounds: [1500.0, 80.0, 1240.0, 820.0],
            display_uuid: Some("external".into()),
            display_origin: [1440.0, 0.0],
            state: WindowState::Maximized,
        };
        let external = DisplayPlacement {
            id: DisplayId::new(7),
            uuid: Some("external".into()),
            bounds: Bounds::new(point(px(-1920.0), px(0.0)), size(px(1920.0), px(1080.0))),
            visible_bounds: Bounds::new(point(px(-1920.0), px(0.0)), size(px(1920.0), px(1040.0))),
        };

        let (bounds, display) =
            restore_window_placement_for_displays(&placement, &[external]).expect("restored");

        assert_eq!(display, Some(DisplayId::new(7)));
        assert_eq!(
            bounds,
            WindowBounds::Maximized(Bounds::new(
                point(px(-1860.0), px(80.0)),
                size(px(1240.0), px(820.0)),
            ))
        );
    }

    #[test]
    fn restored_window_is_rejected_when_its_display_is_disconnected() {
        let placement = WindowPlacement {
            bounds: [1500.0, 80.0, 1240.0, 820.0],
            display_uuid: Some("external".into()),
            display_origin: [1440.0, 0.0],
            state: WindowState::Windowed,
        };
        let builtin = DisplayPlacement {
            id: DisplayId::new(1),
            uuid: Some("builtin".into()),
            bounds: Bounds::new(point(px(0.0), px(0.0)), size(px(1440.0), px(900.0))),
            visible_bounds: Bounds::new(point(px(0.0), px(0.0)), size(px(1440.0), px(860.0))),
        };

        assert!(restore_window_placement_for_displays(&placement, &[builtin]).is_none());
    }
}
