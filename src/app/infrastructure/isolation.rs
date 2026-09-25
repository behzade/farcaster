use std::{
    ffi::OsString,
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub(crate) const ARGUMENT: &str = "--isolated";

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

// App-state isolation only: project files and backend-owned histories stay shared.
// Keep this guard in main until the app and MCP server have shut down.
pub(crate) struct Isolation {
    directory: tempfile::TempDir,
    mcp_addr: String,
}

impl Isolation {
    pub(crate) fn summary(&self) -> String {
        format!(
            "isolated app data={} mcp=http://{}/mcp pid={}; projects and backend histories remain shared",
            self.directory.path().display(),
            self.mcp_addr,
            std::process::id()
        )
    }
}

pub(crate) fn split(arguments: impl Iterator<Item = OsString>) -> (Option<PathBuf>, bool) {
    let mut project = None;
    let mut isolated = false;
    for argument in arguments {
        if argument == ARGUMENT {
            isolated = true;
        } else if project.is_none() {
            project = Some(PathBuf::from(argument));
        }
    }
    (project, isolated)
}

pub(crate) fn is_isolated() -> bool {
    DATA_DIR.get().is_some()
}

pub(crate) fn data_dir() -> Option<&'static Path> {
    DATA_DIR.get().map(PathBuf::as_path)
}

pub(crate) fn window_title(title: &str) -> String {
    if is_isolated() {
        format!("{title} — Isolated app state")
    } else {
        title.to_owned()
    }
}

pub(crate) fn install(source: &Path) -> Result<Isolation, String> {
    if is_isolated() {
        return Err("isolation was already installed".into());
    }
    let mut isolation = prepare(source)?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("reserve isolated MCP listener: {error}"))?;
    isolation.mcp_addr = listener
        .local_addr()
        .map_err(|error| format!("read isolated MCP address: {error}"))?
        .to_string();
    farcaster_mcp_server::install_listener(listener)?;
    DATA_DIR
        .set(isolation.directory.path().to_path_buf())
        .map_err(|_| "isolation was already installed".to_owned())?;
    Ok(isolation)
}

fn prepare(source: &Path) -> Result<Isolation, String> {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = tempfile::Builder::new()
        .permissions(fs::Permissions::from_mode(0o700))
        .prefix("farcaster-isolated-")
        .tempdir()
        .map_err(|error| format!("create isolated app directory: {error}"))?;
    copy_app_state(source, directory.path())?;
    Ok(Isolation {
        directory,
        mcp_addr: String::new(),
    })
}

fn copy_app_state(source: &Path, destination: &Path) -> Result<(), String> {
    let Some(metadata) = metadata_if_present(source)? else {
        return Ok(());
    };
    if !metadata.is_dir() {
        return Err(format!(
            "app data is not a regular directory: {}",
            source.display()
        ));
    }
    let database = source.join("state.sqlite3");
    if require_regular_file(&database)? {
        let snapshot = destination.join("state.sqlite3");
        crate::storage::snapshot_database(&database, &snapshot)?;
        crate::storage::relocate_snapshot_session_locators(&snapshot, source, destination)?;
    }
    // Copy only durable app-owned assets, not logs, transient launch files, or
    // database sidecars. Images are immutable and are published before DB rows.
    for name in ["projects.json", "project-trust.json"] {
        copy_optional_file(&source.join(name), &destination.join(name))?;
    }
    let images = source.join("images");
    if let Some(metadata) = metadata_if_present(&images)? {
        if !metadata.is_dir() {
            return Err(format!(
                "images are not a regular directory: {}",
                images.display()
            ));
        }
        let target = destination.join("images");
        fs::create_dir(&target).map_err(|error| format!("create {}: {error}", target.display()))?;
        for entry in
            fs::read_dir(&images).map_err(|error| format!("read {}: {error}", images.display()))?
        {
            let entry = entry.map_err(|error| format!("read {}: {error}", images.display()))?;
            // Atomic image publication can leave an in-flight temporary file.
            let name = entry.file_name();
            if name.to_str().is_some_and(|name| {
                name.len() == 64
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            }) {
                copy_optional_file(&entry.path(), &target.join(name))?;
            }
        }
    }
    Ok(())
}

fn metadata_if_present(path: &Path) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("inspect {}: {error}", path.display())),
    }
}

fn require_regular_file(path: &Path) -> Result<bool, String> {
    match metadata_if_present(path)? {
        None => Ok(false),
        Some(metadata) if metadata.is_file() => Ok(true),
        Some(_) => Err(format!(
            "app data is not a regular file: {}",
            path.display()
        )),
    }
}

fn copy_optional_file(source: &Path, destination: &Path) -> Result<(), String> {
    if require_regular_file(source)? {
        fs::copy(source, destination)
            .map_err(|error| format!("copy {}: {error}", source.display()))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "isolation_tests.rs"]
mod tests;
