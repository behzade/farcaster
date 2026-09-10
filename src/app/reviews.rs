//! Editor-neutral, advisory review locations carried in MCP tool results.
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Review {
    pub title: String,
    pub items: Vec<ReviewLocation>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReviewLocation {
    pub path: String,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    pub note: String,
}

impl Review {
    pub(crate) fn validate(&self) -> Result<(), String> {
        bounded_text(&self.title, 200, "title")?;
        if self.items.is_empty() || self.items.len() > 100 {
            return Err("Review must contain 1–100 locations".into());
        }
        for item in &self.items {
            bounded_text(&item.path, 4096, "path")?;
            bounded_text(&item.note, 1000, "note")?;
            let path = Path::new(&item.path);
            if path.is_absolute()
                || !path
                    .components()
                    .all(|part| matches!(part, Component::Normal(_)))
            {
                return Err("Review paths must be project-relative without traversal".into());
            }
            match (item.start_line, item.end_line) {
                (Some(0), _) | (_, Some(0)) | (None, Some(_)) => {
                    return Err("Review ranges require a positive start line".into());
                }
                (Some(start), Some(end)) if end < start => {
                    return Err("Review end line must not precede start line".into());
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn bounded_text(text: &str, max: usize, field: &str) -> Result<(), String> {
    if text.trim().is_empty() || text.len() > max || text.chars().any(char::is_control) {
        return Err(format!(
            "Review {field} must be nonempty, at most {max} bytes, without control characters"
        ));
    }
    Ok(())
}

/// Resolve existing ancestors too, so missing files beneath escaping symlinks
/// cannot become editor targets. Missing paths remain useful advisory entries.
pub(crate) fn resolve_path(project: &Path, relative: &str) -> Result<PathBuf, String> {
    let root = project.canonicalize().map_err(|error| error.to_string())?;
    let candidate = root.join(relative);
    let mut ancestor = candidate.as_path();
    loop {
        match std::fs::symlink_metadata(ancestor) {
            Ok(_) => {
                let resolved = ancestor.canonicalize().map_err(|error| error.to_string())?;
                if !resolved.starts_with(&root) {
                    return Err(format!("Review path escapes the project: {relative}"));
                }
                let suffix = candidate
                    .strip_prefix(ancestor)
                    .map_err(|e| e.to_string())?;
                return Ok(if suffix.as_os_str().is_empty() {
                    resolved
                } else {
                    resolved.join(suffix)
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ancestor = ancestor.parent().ok_or_else(|| error.to_string())?;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

#[cfg(test)]
#[path = "reviews_tests.rs"]
mod tests;
