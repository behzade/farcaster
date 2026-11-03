use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};

use semver::Version;
use serde::Deserialize;

#[derive(Deserialize)]
struct VersionCache {
    latest_version: String,
    dismissed_version: Option<String>,
}

pub(super) fn notice(home: &Path, user_agent: &str) -> Option<String> {
    // The interactive CLI owns this cache, including install-channel checks and dismissals.
    let path = home.join("version.json");
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > 4096 {
        return None;
    }
    let cache = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    let (current, latest) = available_update(user_agent, cache)?;

    // Opening another tab for the same installation should not repeat the notice.
    static SHOWN: LazyLock<Mutex<HashMap<PathBuf, Version>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    let mut shown = SHOWN.lock().ok()?;
    if shown.get(home) == Some(&latest) {
        return None;
    }
    shown.insert(home.to_owned(), latest.clone());
    Some(format!(
        "Codex {latest} is available (running {current}). Open the Codex CLI to update."
    ))
}

fn available_update(user_agent: &str, cache: VersionCache) -> Option<(Version, Version)> {
    let (_, rest) = user_agent.split_once('/')?;
    let current = Version::parse(rest.split_whitespace().next()?).ok()?;
    let latest = Version::parse(&cache.latest_version).ok()?;
    if !current.pre.is_empty()
        || !current.build.is_empty()
        || current == Version::new(0, 0, 0)
        || !latest.pre.is_empty()
        || !latest.build.is_empty()
        || latest <= current
        || cache.dismissed_version.as_ref() == Some(&cache.latest_version)
    {
        return None;
    }
    Some((current, latest))
}

#[cfg(test)]
#[path = "updates_tests.rs"]
mod tests;
