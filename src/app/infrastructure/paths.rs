use std::path::PathBuf;

pub(crate) fn data_dir() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("FARCASTER_DATA_DIR") {
        return absolute(PathBuf::from(path));
    }

    let path = match std::env::var_os("XDG_DATA_HOME") {
        Some(path) => PathBuf::from(path),
        None => home_dir()?.join(".local/share"),
    }
    .join("farcaster");

    absolute(path)
}

fn home_dir() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is not set and FARCASTER_DATA_DIR is not set".to_owned())
}

fn absolute(path: PathBuf) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path)
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .map_err(|error| format!("resolve Farcaster data directory: {error}"))
    }
}
