//! Argument validation and native window setup.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};

use crate::{
    app::{
        COMPOSER_KEY_CONTEXT, DismissSurface, OVERLAY_KEY_CONTEXT, PiApp, QuitApplication,
        SubmitFollowUp,
    },
    assets::AppAssets,
    theme::{THEME, install_component_theme},
};
use gpui::{
    App, AppContext as _, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, size,
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
    #[error("the bundled Lilex fonts could not be loaded")]
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
            cx.on_action(quit);
            cx.bind_keys([
                KeyBinding::new("cmd-q", QuitApplication, None),
                KeyBinding::new("escape", DismissSurface, Some(OVERLAY_KEY_CONTEXT)),
                KeyBinding::new(
                    "tab",
                    SubmitFollowUp,
                    Some(&format!("{COMPOSER_KEY_CONTEXT} > Input")),
                ),
            ]);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let bounds = Bounds::centered(
                None,
                size(THEME.layout.window_width, THEME.layout.window_height),
                cx,
            );
            let result = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Pi".into()),
                        ..TitlebarOptions::default()
                    }),
                    app_id: Some("local.pi.gpui".into()),
                    ..WindowOptions::default()
                },
                move |window, cx| {
                    let app = cx.new(|cx| PiApp::new(project.clone(), window, cx));
                    cx.new(|cx| gpui_component::Root::new(app, window, cx))
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

fn quit(_: &QuitApplication, cx: &mut App) {
    cx.quit();
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
}
