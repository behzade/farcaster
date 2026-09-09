use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Duration};

#[cfg(target_os = "linux")]
use std::{fs, sync::Arc};

use crate::{
    app::FarcasterApp,
    app::infrastructure::persistence::{StateStore, WindowPlacement, WindowState},
    app::ui::theme::{THEME, install_component_theme},
    app::ui::{
        assets::AppAssets,
        keybindings,
        keyboard::{ClipboardCopyAlias, ClipboardPasteAlias},
    },
    app::views::dialogs::startup_trust::ProjectTrustView,
    app::workspace::{CycleWorkspaceBackward, CycleWorkspaceForward},
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
    #[error("the Farcaster window could not open: {0}")]
    NativeWindow(String),
}

pub(crate) fn resolve_project(path: Option<PathBuf>) -> Result<PathBuf, LaunchError> {
    let path = match path.or_else(crate::app::project::registry::most_recent) {
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

fn update_app(
    app: &RefCell<Option<WeakEntity<FarcasterApp>>>,
    cx: &mut App,
    update: impl FnOnce(&mut FarcasterApp, &mut Window, &mut Context<FarcasterApp>),
) {
    if let Some(app) = app.borrow().clone() {
        let _ = app.update_in(cx, update);
    }
}

pub(crate) fn run(
    project: PathBuf,
    workgraph_updates: async_channel::Receiver<()>,
    worker_updates: async_channel::Receiver<()>,
) -> Result<(), LaunchError> {
    #[cfg(target_os = "linux")]
    install_linux_desktop_identity();

    let trust_timing =
        crate::app::infrastructure::performance::StartupTiming::new("launch.project_trust");
    let startup_trust =
        crate::app::project::trust::startup_trust(&project).map_err(LaunchError::ProjectTrust)?;
    drop(trust_timing);
    let failure = Rc::new(RefCell::new(None));
    let failure_in_app = failure.clone();
    gpui_platform::application()
        .with_assets(AppAssets)
        .run(move |cx: &mut App| {
            cx.set_app_identity("io.github.behzade.farcaster", "Farcaster");
            gpui_component::init(cx);
            let fonts_timing =
                crate::app::infrastructure::performance::StartupTiming::new("launch.load_fonts");
            if AppAssets.load_fonts(cx).is_err() {
                *failure_in_app.borrow_mut() = Some(LaunchError::BundledFonts);
                quit_after_start(cx);
                return;
            }
            drop(fonts_timing);
            install_component_theme(cx);
            let notification_app: Rc<RefCell<Option<WeakEntity<FarcasterApp>>>> =
                Rc::new(RefCell::new(None));
            super::quit::install(
                notification_app.clone(),
                FarcasterApp::request_application_quit,
                cx,
            );
            let cycle_forward_app = notification_app.clone();
            cx.on_action(move |_: &CycleWorkspaceForward, cx| {
                update_app(&cycle_forward_app, cx, |app, window, cx| {
                    app.cycle_workspace_surface(true, window, cx);
                });
            });
            let cycle_backward_app = notification_app.clone();
            cx.on_action(move |_: &CycleWorkspaceBackward, cx| {
                update_app(&cycle_backward_app, cx, |app, window, cx| {
                    app.cycle_workspace_surface(false, window, cx);
                });
            });
            let copy_app = notification_app.clone();
            cx.on_action(move |_: &ClipboardCopyAlias, cx| {
                update_app(&copy_app, cx, |app, window, cx| {
                    app.handle_clipboard_alias(false, window, cx);
                });
            });
            let paste_app = notification_app.clone();
            cx.on_action(move |_: &ClipboardPasteAlias, cx| {
                update_app(&paste_app, cx, |app, window, cx| {
                    app.handle_clipboard_alias(true, window, cx);
                });
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
            cx.bind_keys(keybindings::bindings());
            #[cfg(target_os = "macos")]
            super::menus::install(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let placement_timing = crate::app::infrastructure::performance::StartupTiming::new(
                "launch.restore_window",
            );
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
            drop(placement_timing);
            let window_options = WindowOptions {
                window_bounds: Some(window_bounds),
                display_id,
                titlebar: Some(TitlebarOptions {
                    title: Some("Farcaster".into()),
                    ..TitlebarOptions::default()
                }),
                app_id: Some("io.github.behzade.farcaster".into()),
                ..WindowOptions::default()
            };
            #[cfg(target_os = "linux")]
            let window_options = {
                let mut window_options = window_options;
                window_options.icon = image::load_from_memory(include_bytes!(
                    "../../../assets/icons/app/icon_256x256.png"
                ))
                .ok()
                .map(image::DynamicImage::into_rgba8)
                .map(Arc::new);
                window_options
            };
            let open_window_timing =
                crate::app::infrastructure::performance::StartupTiming::new("launch.open_window");
            let result = cx.open_window(window_options, move |window, cx| {
                super::quit::install_window(window, cx);
                let launch = cx.new(|cx| {
                    ProjectTrustView::new(
                        project.clone(),
                        startup_trust,
                        notification_app.clone(),
                        workgraph_updates.clone(),
                        worker_updates.clone(),
                        window,
                        cx,
                    )
                });
                cx.new(|cx| gpui_component::Root::new(launch, window, cx))
            });
            drop(open_window_timing);
            if let Err(error) = result {
                *failure_in_app.borrow_mut() =
                    Some(LaunchError::NativeWindow(format!("{error:#}")));
                quit_after_start(cx);
                return;
            }
            cx.activate(true);
        });
    match failure.borrow_mut().take() {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn quit_after_start(cx: &mut App) {
    // Linux invokes the launch callback before calloop::run, which resets its
    // stop signal. Queue shutdown on the foreground executor so it is not lost.
    cx.spawn(async |cx| {
        let _ = cx.update(|cx| cx.quit());
    })
    .detach();
}

#[cfg(target_os = "linux")]
fn install_linux_desktop_identity() {
    const APP_ID: &str = "io.github.behzade.farcaster";
    const ICON: &[u8] = include_bytes!("../../../assets/icons/app/icon_256x256.png");

    let Ok(data_home) = crate::app::infrastructure::paths::user_data_home() else {
        return;
    };
    let icon_dir = data_home.join("icons/hicolor/256x256/apps");
    let applications_dir = data_home.join("applications");
    if fs::create_dir_all(&icon_dir).is_err() || fs::create_dir_all(&applications_dir).is_err() {
        return;
    }
    let icon = icon_dir.join(format!("{APP_ID}.png"));
    if fs::write(&icon, ICON).is_err() {
        return;
    }

    let executable = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok());
    let Some(executable) = executable else {
        return;
    };
    let executable = desktop_exec_path(&executable.to_string_lossy());
    let icon = icon.to_string_lossy();
    let desktop_entry = format!(
        "[Desktop Entry]\n\
         Categories=Development;\n\
         Comment=Native desktop client for coding agents\n\
         Exec=\"{executable}\"\n\
         Icon={icon}\n\
         Name=Farcaster\n\
         StartupWMClass={APP_ID}\n\
         Terminal=false\n\
         Type=Application\n"
    );
    let _ = fs::write(
        applications_dir.join(format!("{APP_ID}.desktop")),
        desktop_entry,
    );
}

#[cfg(target_os = "linux")]
fn desktop_exec_path(path: &str) -> String {
    path.chars().fold(String::new(), |mut escaped, character| {
        if matches!(character, '\\' | '"' | '`' | '$') {
            escaped.push('\\');
        }
        if character == '%' {
            escaped.push('%');
        }
        escaped.push(character);
        escaped
    })
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
#[path = "launch_tests.rs"]
mod tests;
