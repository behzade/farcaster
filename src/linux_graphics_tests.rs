use super::*;

#[test]
fn native_linux_disables_dzn_without_overriding_driver_configuration() {
    assert!(should_disable_dzn(false, |_| false));
    assert!(!should_disable_dzn(true, |_| false));
    for configured_name in VULKAN_DRIVER_CONFIGURATION {
        assert!(!should_disable_dzn(false, |name| name == configured_name));
    }
}

#[test]
fn recognizes_wsl_kernel_versions() {
    assert!(kernel_version_reports_wsl(
        "5.15.90.1-microsoft-standard-WSL2"
    ));
    assert!(kernel_version_reports_wsl("4.4.0-Microsoft"));
    assert!(!kernel_version_reports_wsl("7.1.4"));
}

#[test]
fn non_nixos_appimages_keep_their_environment() {
    let directory = tempfile::tempdir().unwrap();
    assert!(
        appimage_environment(&directory.path().join("absent"), false, |_| None)
            .unwrap()
            .is_empty()
    );
}

fn driver_fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for name in [
        "share/vulkan/icd.d/radeon_icd.json",
        "share/vulkan/icd.d/dzn_icd.json",
        "share/vulkan/icd.d/ignored.txt",
        "share/glvnd/egl_vendor.d/50_mesa.json",
    ] {
        let path = root.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "{}").unwrap();
    }
    root
}

#[test]
fn nixos_appimage_finds_host_manifests_without_native_linux_dzn() {
    let root = driver_fixture();
    let environment = appimage_environment(root.path(), false, |_| None).unwrap();
    let vulkan = &environment
        .iter()
        .find(|(key, _)| *key == "VK_ICD_FILENAMES")
        .unwrap()
        .1;
    assert_eq!(
        std::env::split_paths(vulkan).collect::<Vec<_>>(),
        [root.path().join("share/vulkan/icd.d/radeon_icd.json")]
    );
    let egl = &environment
        .iter()
        .find(|(key, _)| *key == "__EGL_VENDOR_LIBRARY_FILENAMES")
        .unwrap()
        .1;
    assert_eq!(
        std::env::split_paths(egl).collect::<Vec<_>>(),
        [root.path().join("share/glvnd/egl_vendor.d/50_mesa.json")]
    );
}

#[test]
fn wsl_keeps_dzn_available() {
    let root = driver_fixture();
    let environment = appimage_environment(root.path(), true, |_| None).unwrap();
    let vulkan = &environment
        .iter()
        .find(|(key, _)| *key == "VK_ICD_FILENAMES")
        .unwrap()
        .1;
    assert!(std::env::split_paths(vulkan).any(|path| path.ends_with("dzn_icd.json")));
}

#[test]
fn host_manifest_defaults_respect_explicit_driver_configuration() {
    let root = driver_fixture();
    for key in VULKAN_DRIVER_CONFIGURATION {
        let environment = appimage_environment(root.path(), false, |name| {
            (name == key).then(|| "explicit".into())
        })
        .unwrap();
        assert!(
            !environment
                .iter()
                .any(|(name, _)| *name == "VK_ICD_FILENAMES"),
            "{key}"
        );
    }
    for key in [
        "__EGL_VENDOR_LIBRARY_FILENAMES",
        "__EGL_VENDOR_LIBRARY_DIRS",
    ] {
        let environment = appimage_environment(root.path(), false, |name| {
            (name == key).then(|| "explicit".into())
        })
        .unwrap();
        assert!(
            !environment
                .iter()
                .any(|(name, _)| *name == "__EGL_VENDOR_LIBRARY_FILENAMES"),
            "{key}"
        );
    }
}

#[test]
fn wayland_resolution_uses_only_a_resolved_client_library() {
    assert_eq!(
        wayland_dependency(
            "\tlibwayland-client.so.0 => /host/lib/libwayland-client.so.0 (0x123)\n"
        ),
        Some("/host/lib/libwayland-client.so.0")
    );
    for output in [
        "libwayland-client.so.0 => not found",
        "libwayland-client.so.0 => relative/path (0x123)",
        "libwayland-server.so.0 => /host/lib/libwayland-server.so.0 (0x123)",
    ] {
        assert_eq!(wayland_dependency(output), None);
    }
}

#[test]
fn host_wayland_preload_preserves_entries_and_converges_after_relaunch() {
    let host = "/host/lib/libwayland-client.so.0";
    let preload = preload_host_wayland(Some("/custom/a.so /custom/b.so".into()), host).unwrap();
    assert_eq!(
        preload,
        OsString::from(format!("{host}:/custom/a.so /custom/b.so"))
    );
    assert!(preload_host_wayland(Some(preload), host).is_none());
    assert!(preload_host_wayland(Some(format!("/custom/a.so {host}").into()), host).is_none());
}

#[test]
fn manifest_defaults_converge_after_relaunch() {
    let root = driver_fixture();
    let environment = appimage_environment(root.path(), false, |_| None).unwrap();
    let second = appimage_environment(root.path(), false, |name| {
        environment
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.clone())
    })
    .unwrap();
    assert!(second.is_empty());
}
