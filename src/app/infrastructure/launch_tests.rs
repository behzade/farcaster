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

#[cfg(target_os = "linux")]
#[test]
fn linux_desktop_native_launch_does_not_create_a_user_entry()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let data_home = directory.path().join("share");

    install_linux_desktop_identity_at(&data_home, None, &[]);

    assert!(!data_home.exists());
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn linux_desktop_native_launch_preserves_a_managed_entry() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempdir()?;
    let packaged_entry = directory.path().join("packaged.desktop");
    let contents = "[Desktop Entry]\nExec=/nix/store/package/bin/farcaster %f\n";
    fs::write(&packaged_entry, contents)?;
    let applications = directory.path().join("applications");
    fs::create_dir(&applications)?;
    let entry = applications.join("io.github.behzade.farcaster.desktop");
    std::os::unix::fs::symlink(&packaged_entry, &entry)?;

    install_linux_desktop_identity_at(directory.path(), None, &[]);

    assert_eq!(fs::read_link(&entry)?, packaged_entry);
    assert_eq!(fs::read_to_string(&entry)?, contents);
    assert!(!directory.path().join("icons").exists());
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn linux_desktop_appimage_launch_registers_its_launcher_and_icon()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let appimage = directory.path().join("Farcaster 100%.AppImage");

    install_linux_desktop_identity_at(directory.path(), Some(appimage), &[]);

    let entry = directory
        .path()
        .join("applications/io.github.behzade.farcaster.desktop");
    let contents = fs::read_to_string(&entry)?;
    assert!(contents.contains(&format!(
        "Exec=\"{}/Farcaster 100%%.AppImage\"\n",
        directory.path().display()
    )));
    let icon = directory
        .path()
        .join("icons/hicolor/256x256/apps/io.github.behzade.farcaster.png");
    assert_eq!(
        fs::read(&icon)?,
        include_bytes!("../../../assets/icons/app/icon_256x256.png")
    );
    assert!(contents.contains(&format!("Icon={}\n", icon.display())));
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn linux_desktop_appimage_launch_updates_an_older_appimage_entry()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let old = directory.path().join("Farcaster-old.AppImage");
    let new = directory.path().join("Farcaster-new.AppImage");

    install_linux_desktop_identity_at(directory.path(), Some(old), &[]);
    install_linux_desktop_identity_at(directory.path(), Some(new.clone()), &[]);

    let entry = directory
        .path()
        .join("applications/io.github.behzade.farcaster.desktop");
    let contents = fs::read_to_string(entry)?;
    assert!(contents.contains(&format!("Exec=\"{}\"\n", new.display())));
    assert!(!contents.contains("Farcaster-old.AppImage"));
    Ok(())
}

#[cfg(target_os = "linux")]
fn legacy_linux_desktop_entry(data_home: &Path, executable: &str) -> String {
    // Keep the pre-migration format independent of the production formatter.
    format!(
        "[Desktop Entry]\nCategories=Development;\nComment=Native desktop client for coding agents\nExec=\"{executable}\"\nIcon={}/icons/hicolor/256x256/apps/io.github.behzade.farcaster.png\nName=Farcaster\nStartupWMClass=io.github.behzade.farcaster\nTerminal=false\nType=Application\n",
        data_home.display()
    )
}

#[cfg(target_os = "linux")]
#[test]
fn linux_desktop_native_upgrade_removes_only_generated_package_entries()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let data_home = directory.path().join("user");
    let packaged_home = directory.path().join("package");
    let entry_name = "applications/io.github.behzade.farcaster.desktop";
    fs::create_dir_all(data_home.join("applications"))?;
    fs::create_dir_all(packaged_home.join("applications"))?;
    let packaged = packaged_home.join(entry_name);
    let packaged_contents = "[Desktop Entry]\nType=Application\nExec=farcaster %f\n";
    fs::write(&packaged, packaged_contents)?;
    let entry = data_home.join(entry_name);

    for executable in [
        "/nix/store/old-farcaster-bin-0.4.2/lib/farcaster/farcaster",
        "/nix/store/old-farcaster-0.4.2/bin/farcaster",
        "/nix/store/old-farcaster-0.4.2/bin/.farcaster-wrapped",
        "/usr/lib/farcaster/farcaster",
        "/usr/bin/farcaster",
        "/usr/local/bin/farcaster",
    ] {
        fs::write(&entry, legacy_linux_desktop_entry(&data_home, executable))?;
        install_linux_desktop_identity_at(&data_home, None, std::slice::from_ref(&packaged_home));
        assert!(!entry.exists(), "old native launcher remains: {executable}");
        assert_eq!(fs::read_to_string(&packaged)?, packaged_contents);
    }
    // Repeated launches have nothing left to migrate.
    install_linux_desktop_identity_at(&data_home, None, &[packaged_home]);
    assert!(!entry.exists());
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn linux_desktop_native_upgrade_preserves_entries_without_a_replacement()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let entry = directory
        .path()
        .join("applications/io.github.behzade.farcaster.desktop");
    fs::create_dir(entry.parent().unwrap())?;
    let contents = legacy_linux_desktop_entry(directory.path(), "/usr/lib/farcaster/farcaster");
    fs::write(&entry, &contents)?;

    install_linux_desktop_identity_at(directory.path(), None, &[]);
    assert_eq!(fs::read_to_string(&entry)?, contents);
    // XDG_DATA_DIRS can include XDG_DATA_HOME or a symlink back to it.
    let alias = directory.path().join("alias");
    std::os::unix::fs::symlink(directory.path(), &alias)?;
    install_linux_desktop_identity_at(directory.path(), None, &[alias]);
    assert_eq!(fs::read_to_string(&entry)?, contents);
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn linux_desktop_native_upgrade_preserves_custom_appimage_and_managed_entries()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let data_home = directory.path().join("user");
    let packaged_home = directory.path().join("package");
    let entry_name = "applications/io.github.behzade.farcaster.desktop";
    fs::create_dir_all(data_home.join("applications"))?;
    fs::create_dir_all(packaged_home.join("applications"))?;
    fs::write(
        packaged_home.join(entry_name),
        "[Desktop Entry]\nExec=farcaster %f\n",
    )?;
    let entry = data_home.join(entry_name);
    let legacy = legacy_linux_desktop_entry(&data_home, "/usr/lib/farcaster/farcaster");

    for contents in [
        legacy.replace("Name=Farcaster", "Name=My Farcaster"),
        format!("{legacy}NoDisplay=true\n"),
        legacy_linux_desktop_entry(&data_home, "/home/user/bin/farcaster"),
        legacy_linux_desktop_entry(&data_home, "/nix/store/other-package/bin/farcaster"),
        legacy_linux_desktop_entry(&data_home, "/usr/bin/.farcaster-wrapped"),
        legacy_linux_desktop_entry(&data_home, "/home/user/Downloads/Farcaster.AppImage"),
        legacy_linux_desktop_entry(&data_home, "/home/user/Downloads/farcaster"),
    ] {
        fs::write(&entry, &contents)?;
        install_linux_desktop_identity_at(&data_home, None, std::slice::from_ref(&packaged_home));
        assert_eq!(fs::read_to_string(&entry)?, contents);
    }
    fs::remove_file(&entry)?;
    // Even a symlink to the exact old generated format belongs to its manager.
    let managed = directory.path().join("managed.desktop");
    fs::write(&managed, &legacy)?;
    std::os::unix::fs::symlink(&managed, &entry)?;
    install_linux_desktop_identity_at(&data_home, None, &[packaged_home]);
    assert_eq!(fs::read_link(&entry)?, managed);
    assert_eq!(fs::read_to_string(&entry)?, legacy);
    Ok(())
}
