//! Argument validation and native window setup.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use gpui::{
    App, AppContext as _, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, size,
};
use gpui_component_assets::Assets;

use crate::{
    app::{DismissSurface, OVERLAY_KEY_CONTEXT, PiApp},
    theme::{THEME, install_component_theme},
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
    #[error("the native Pi window could not open")]
    NativeWindow,
}

pub(crate) fn resolve_project(path: Option<PathBuf>) -> Result<PathBuf, LaunchError> {
    let path = match path {
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
    let failed = Arc::new(AtomicBool::new(false));
    let failed_in_app = failed.clone();
    gpui_platform::application()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            install_component_theme(cx);
            cx.bind_keys([KeyBinding::new(
                "escape",
                DismissSurface,
                Some(OVERLAY_KEY_CONTEXT),
            )]);
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
                failed_in_app.store(true, Ordering::Release);
                cx.quit();
                return;
            }
            cx.activate(true);
        });
    if failed.load(Ordering::Acquire) {
        Err(LaunchError::NativeWindow)
    } else {
        Ok(())
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
}
