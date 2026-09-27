use std::path::{Path, PathBuf};

use farcaster_contracts::Backend;
use serde::{Deserialize, Serialize};

/// A positive identity allocated by the application state store.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "i64", into = "i64")]
pub struct AppSessionId(i64);

impl AppSessionId {
    pub fn new(value: i64) -> Option<Self> {
        (value > 0).then_some(Self(value))
    }

    pub fn get(self) -> i64 {
        self.0
    }

    /// Stable opaque key for consumers which store session ownership as text.
    pub fn to_key(self) -> String {
        format!("app-session:{}", self.0)
    }

    pub fn from_key(key: &str) -> Option<Self> {
        let id = Self::new(key.strip_prefix("app-session:")?.parse().ok()?)?;
        (id.to_key() == key).then_some(id)
    }
}

impl TryFrom<i64> for AppSessionId {
    type Error = &'static str;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::new(value).ok_or("application session ID must be positive")
    }
}

impl From<AppSessionId> for i64 {
    fn from(value: AppSessionId) -> Self {
        value.get()
    }
}

/// Exact routing identity. Binding must explicitly replace a locator key with
/// its application key; these variants never compare equal implicitly.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum SessionKey {
    App(AppSessionId),
    Locator {
        harness: Backend,
        profile_id: Option<String>,
        path: PathBuf,
    },
}

/// A backend's native ID is unique only within this scope. Use application IDs
/// or locators for routing when available, and resolve legacy native IDs explicitly.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct NativeSessionIdentity {
    pub project: PathBuf,
    pub harness: Backend,
    pub profile_id: Option<String>,
    pub id: String,
}

/// Compatibility fallback for synthetic locators. Native session paths do not
/// encode a profile; their explicit persisted profile remains authoritative.
pub fn profile_id_from_locator(path: &Path) -> Option<String> {
    let backend = path.parent()?;
    let id = backend.parent()?;
    (id.parent()?.file_name()? == "profiles")
        .then(|| id.file_name()?.to_str())
        .flatten()
        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
        .map(str::to_owned)
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
