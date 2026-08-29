//! Project-local discovery configuration for agent MCP clients.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

const CONFIG_NAME: &str = ".mcp.json";
const SERVER_NAME: &str = "farcaster";
const SERVER_URL: &str = "http://127.0.0.1:8765/mcp";
static CONFIG_WRITE: Mutex<()> = Mutex::new(());

pub(crate) fn ensure_project_config(project: &Path) -> Result<(), String> {
    let _guard = CONFIG_WRITE
        .lock()
        .map_err(|_| "Farcaster MCP configuration lock is unavailable".to_owned())?;
    let path = project.join(CONFIG_NAME);
    let (mut config, permissions) = match fs::read(&path) {
        Ok(bytes) => {
            let permissions = fs::metadata(&path)
                .map_err(|error| format!("read {} metadata: {error}", path.display()))?
                .permissions();
            let config = serde_json::from_slice::<serde_json::Value>(&bytes)
                .map_err(|error| format!("decode {}: {error}", path.display()))?;
            (config, Some(permissions))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (serde_json::json!({}), None),
        Err(error) => return Err(format!("read {}: {error}", path.display())),
    };

    let root = config
        .as_object_mut()
        .ok_or_else(|| format!("{} must contain a JSON object", path.display()))?;
    let servers = root
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| format!("{}.mcpServers must contain a JSON object", path.display()))?;
    let entry = serde_json::json!({ "url": SERVER_URL });
    if servers.get(SERVER_NAME) == Some(&entry) {
        return Ok(());
    }
    servers.insert(SERVER_NAME.into(), entry);
    write_atomic(&path, &config, permissions)
}

fn write_atomic(
    path: &Path,
    config: &serde_json::Value,
    permissions: Option<fs::Permissions>,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(config)
        .map_err(|error| format!("encode {}: {error}", path.display()))?;
    bytes.push(b'\n');
    let temporary = temporary_path(path);
    fs::write(&temporary, bytes)
        .map_err(|error| format!("write {}: {error}", temporary.display()))?;
    if let Some(permissions) = permissions {
        fs::set_permissions(&temporary, permissions)
            .map_err(|error| format!("preserve {} permissions: {error}", path.display()))?;
    }
    fs::rename(&temporary, path).map_err(|error| {
        let _remove = fs::remove_file(&temporary);
        format!(
            "replace {} with {}: {error}",
            path.display(),
            temporary.display()
        )
    })
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(format!(".{}.tmp", std::process::id()));
    PathBuf::from(temporary)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_config(path: &Path) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    }

    #[test]
    fn creates_shared_project_config() -> Result<(), Box<dyn std::error::Error>> {
        let project = tempfile::tempdir()?;
        ensure_project_config(project.path())?;
        assert_eq!(
            read_config(&project.path().join(CONFIG_NAME))?["mcpServers"][SERVER_NAME]["url"],
            SERVER_URL
        );
        Ok(())
    }

    #[test]
    fn preserves_other_servers_and_replaces_stale_farcaster_entry()
    -> Result<(), Box<dyn std::error::Error>> {
        let project = tempfile::tempdir()?;
        let path = project.path().join(CONFIG_NAME);
        fs::write(
            &path,
            br#"{
                "settings": { "keep": true },
                "mcpServers": {
                    "docs": { "url": "https://docs.example/mcp" },
                    "farcaster": { "url": "http://127.0.0.1:1/old" }
                }
            }"#,
        )?;

        ensure_project_config(project.path())?;

        let config = read_config(&path)?;
        assert_eq!(config["settings"]["keep"], true);
        assert_eq!(
            config["mcpServers"]["docs"]["url"],
            "https://docs.example/mcp"
        );
        assert_eq!(config["mcpServers"][SERVER_NAME]["url"], SERVER_URL);
        Ok(())
    }

    #[test]
    fn refuses_to_overwrite_invalid_existing_config() -> Result<(), Box<dyn std::error::Error>> {
        let project = tempfile::tempdir()?;
        let path = project.path().join(CONFIG_NAME);
        fs::write(&path, b"not json")?;

        let error = ensure_project_config(project.path()).expect_err("invalid config must fail");

        assert!(error.contains("decode"));
        assert_eq!(fs::read(&path)?, b"not json");
        Ok(())
    }
}
