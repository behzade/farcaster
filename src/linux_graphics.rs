//! Set up graphics before loading drivers, including AppImage/NixOS interoperability.
use std::{ffi::OsString, path::Path, process::Command};

const VULKAN_DRIVER_CONFIGURATION: [&str; 5] = [
    "VK_DRIVER_FILES",
    "VK_ICD_FILENAMES",
    "VK_ADD_DRIVER_FILES",
    "VK_LOADER_DRIVERS_SELECT",
    "VK_LOADER_DRIVERS_DISABLE",
];

type Environment = Vec<(&'static str, OsString)>;

pub(crate) fn relaunch() -> Result<(), String> {
    use std::os::unix::process::CommandExt as _;

    let is_wsl = std::env::var_os("WSL_INTEROP").is_some()
        || std::env::var_os("WSL_DISTRO_NAME").is_some()
        || ["/proc/sys/kernel/osrelease", "/proc/version"]
            .into_iter()
            .any(|path| {
                std::fs::read_to_string(path)
                    .is_ok_and(|version| kernel_version_reports_wsl(&version))
            });
    let mut environment = Vec::new();
    if std::env::var_os("APPIMAGE").is_some() {
        environment = appimage_environment(Path::new("/run/opengl-driver"), is_wsl, |name| {
            std::env::var_os(name)
        })?;
    }
    if should_disable_dzn(is_wsl, |name| std::env::var_os(name).is_some())
        && !environment
            .iter()
            .any(|(name, _)| *name == "VK_ICD_FILENAMES")
    {
        environment.push(("VK_LOADER_DRIVERS_DISABLE", "*dzn*".into()));
    }
    if environment.is_empty() {
        return Ok(());
    }

    let executable = std::env::current_exe()
        .map_err(|error| format!("resolve farcaster executable for graphics setup: {error}"))?;
    let error = Command::new(executable)
        .args(std::env::args_os().skip(1))
        .envs(environment)
        .exec();
    Err(format!(
        "relaunch farcaster with host graphics setup: {error}"
    ))
}

fn appimage_environment(
    driver_root: &Path,
    is_wsl: bool,
    value: impl Fn(&str) -> Option<OsString>,
) -> Result<Environment, String> {
    let mut environment = Vec::new();
    if !driver_root.is_dir() {
        return Ok(environment);
    }

    // Ubuntu-built loaders do not search NixOS's driver profile. Use the older
    // variable understood by the bundled Vulkan loader, without replacing user policy.
    if !VULKAN_DRIVER_CONFIGURATION
        .iter()
        .any(|name| value(name).is_some())
    {
        let manifests = manifests(&driver_root.join("share/vulkan/icd.d"), !is_wsl)?;
        if !manifests.is_empty() {
            environment.push(("VK_ICD_FILENAMES", manifests));
        }
    }
    if value("__EGL_VENDOR_LIBRARY_FILENAMES").is_none()
        && value("__EGL_VENDOR_LIBRARY_DIRS").is_none()
    {
        let manifests = manifests(&driver_root.join("share/glvnd/egl_vendor.d"), false)?;
        if !manifests.is_empty() {
            environment.push(("__EGL_VENDOR_LIBRARY_FILENAMES", manifests));
        }
    }

    // Host Mesa can require Wayland symbols newer than those in the AppImage.
    // Resolve its own dependency with bundle search paths removed, then preload
    // that ABI-compatible library before any graphics libraries are opened.
    let mesa = driver_root.join("lib/libEGL_mesa.so.0");
    if mesa.is_file() {
        let output = Command::new("ldd")
            .arg(&mesa)
            .env_remove("LD_LIBRARY_PATH")
            .env_remove("LD_PRELOAD")
            .output()
            .map_err(|error| {
                format!(
                    "resolve host Wayland library for {}: {error}",
                    mesa.display()
                )
            })?;
        if !output.status.success() {
            return Err(format!(
                "resolve host Wayland library for {}: ldd failed",
                mesa.display()
            ));
        }
        if let Some(library) = wayland_dependency(&String::from_utf8_lossy(&output.stdout)) {
            if let Some(preload) = preload_host_wayland(value("LD_PRELOAD"), library) {
                environment.push(("LD_PRELOAD", preload));
            }
        }
    }
    Ok(environment)
}

fn manifests(directory: &Path, exclude_dzn: bool) -> Result<OsString, String> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(OsString::new()),
        Err(error) => {
            return Err(format!(
                "read graphics manifests {}: {error}",
                directory.display()
            ));
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|error| format!("read graphics manifest: {error}"))?
            .path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
            && path.is_file()
            && !(exclude_dzn
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().contains("dzn")))
        {
            paths.push(path);
        }
    }
    paths.sort();
    std::env::join_paths(paths).map_err(|error| format!("join graphics manifest paths: {error}"))
}

fn wayland_dependency(ldd_output: &str) -> Option<&str> {
    ldd_output.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        if fields.next()? != "libwayland-client.so.0" || fields.next()? != "=>" {
            return None;
        }
        let path = fields.next()?;
        Path::new(path).is_absolute().then_some(path)
    })
}

fn preload_host_wayland(existing: Option<OsString>, library: &str) -> Option<OsString> {
    use std::os::unix::ffi::OsStrExt as _;
    let existing = existing.unwrap_or_default();
    if existing
        .as_bytes()
        .split(|byte| *byte == b':' || byte.is_ascii_whitespace())
        .any(|entry| entry == library.as_bytes())
    {
        return None;
    }
    let mut preload = OsString::from(library);
    if !existing.is_empty() {
        preload.push(":");
        preload.push(existing);
    }
    Some(preload)
}

fn should_disable_dzn(is_wsl: bool, mut environment_is_set: impl FnMut(&str) -> bool) -> bool {
    !is_wsl
        && !VULKAN_DRIVER_CONFIGURATION
            .into_iter()
            .any(&mut environment_is_set)
}

fn kernel_version_reports_wsl(version: &str) -> bool {
    version.to_ascii_lowercase().contains("microsoft")
}

#[cfg(test)]
#[path = "linux_graphics_tests.rs"]
mod tests;
