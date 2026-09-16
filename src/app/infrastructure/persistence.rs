pub(crate) use crate::persistence::*;

use std::path::PathBuf;

#[path = "persistence/transcript.rs"]
mod transcript;

impl StateStore {
    pub(crate) fn open() -> Result<Self, String> {
        let _startup_timing =
            crate::app::infrastructure::performance::StartupTiming::new("db.open_total");
        let _timing = crate::app::infrastructure::performance::OperationTiming::new(
            crate::app::infrastructure::performance::OperationKind::StateDatabase,
            1,
        );
        let path = state_path()?;
        let mut store = Self::open_at(&path)?;
        if let Some(legacy) = legacy_pi_gpui_state_path()
            && legacy != path
            && legacy.is_file()
        {
            store.import_legacy_pi_gpui_state(&legacy)?;
        }
        Ok(store)
    }
}

pub(crate) fn state_path() -> Result<PathBuf, String> {
    crate::app::infrastructure::paths::data_dir().map(|root| root.join("state.sqlite3"))
}

fn legacy_pi_gpui_state_path() -> Option<PathBuf> {
    let root = std::env::var_os("PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".pi/agent")))?;
    let root = if root.is_absolute() {
        root
    } else {
        std::env::current_dir().ok()?.join(root)
    };
    Some(root.join("gui-state.sqlite3"))
}
