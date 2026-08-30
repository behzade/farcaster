use std::{io::Write as _, path::Path};

const SERVER_URL: &str = "http://127.0.0.1:8765/mcp";

pub(crate) struct TransientMcpConfig {
    file: tempfile::NamedTempFile,
}

impl TransientMcpConfig {
    pub(crate) fn create() -> Result<Self, String> {
        let mut file = tempfile::NamedTempFile::new()
            .map_err(|error| format!("create transient MCP configuration: {error}"))?;
        let mut config = serde_json::to_vec(&serde_json::json!({
            "mcpServers": {
                "farcaster": {
                    "url": SERVER_URL,
                    "protocolVersion": "2026-07-28"
                }
            }
        }))
        .map_err(|error| format!("encode MCP configuration: {error}"))?;
        config.push(b'\n');
        file.write_all(&config)
            .map_err(|error| format!("write transient MCP configuration: {error}"))?;
        file.flush()
            .map_err(|error| format!("flush transient MCP configuration: {error}"))?;
        Ok(Self { file })
    }

    pub(crate) fn path(&self) -> &Path {
        self.file.path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_farcaster_through_a_reopenable_transient_file()
    -> Result<(), Box<dyn std::error::Error>> {
        let config = TransientMcpConfig::create()?;
        let path = config.path().to_owned();
        for _ in 0..2 {
            let value = serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path)?)?;
            assert_eq!(value["mcpServers"]["farcaster"]["url"], SERVER_URL);
            assert_eq!(
                value["mcpServers"]["farcaster"]["protocolVersion"],
                "2026-07-28"
            );
        }
        drop(config);
        assert!(!path.exists());
        Ok(())
    }
}
