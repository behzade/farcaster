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
