use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub(crate) enum Backend {
    #[serde(rename = "pi")]
    Pi,
    #[serde(rename = "codex-cli")]
    Codex,
    #[serde(rename = "cursor-cli")]
    Cursor,
    #[serde(rename = "opencode", alias = "opencode2")]
    OpenCode,
    #[serde(rename = "claude")]
    Claude,
    #[serde(rename = "antigravity-acp")]
    Antigravity,
}

impl Backend {
    pub(crate) const ALL: [Self; 6] = [
        Self::Pi,
        Self::Codex,
        Self::Cursor,
        Self::OpenCode,
        Self::Claude,
        Self::Antigravity,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Pi => "pi",
            Self::Codex => "codex-cli",
            Self::Cursor => "cursor-cli",
            Self::OpenCode => "opencode",
            Self::Claude => "claude",
            Self::Antigravity => "antigravity-acp",
        }
    }
}

impl std::str::FromStr for Backend {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|backend| backend.as_str() == value)
            .ok_or_else(|| format!("unsupported backend: {value}"))
    }
}

impl std::fmt::Display for Backend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<Backend> for String {
    fn from(backend: Backend) -> Self {
        backend.as_str().to_owned()
    }
}
