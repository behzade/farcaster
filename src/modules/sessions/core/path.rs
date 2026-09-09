use std::{
    ffi::OsString,
    path::{Component, Path, PathBuf},
};

use path_clean::PathClean as _;

pub(crate) fn normalize_session_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return normalize_lexical(&canonical);
    }
    canonicalize_existing_ancestor(path).unwrap_or_else(|| normalize_lexical(path))
}

pub(crate) fn normalize_lexical(path: &Path) -> PathBuf {
    if path.as_os_str().is_empty() {
        PathBuf::new()
    } else {
        path.clean()
    }
}

fn canonicalize_existing_ancestor(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let mut ancestor = path.to_path_buf();
    let mut suffix = Vec::<OsString>::new();
    loop {
        if let Ok(canonical) = ancestor.canonicalize() {
            let rebuilt = suffix
                .iter()
                .rev()
                .fold(normalize_lexical(&canonical), |path, component| {
                    path.join(component)
                });
            return Some(normalize_lexical(&rebuilt));
        }
        let component = ancestor.components().next_back()?;
        if matches!(component, Component::RootDir | Component::Prefix(_)) {
            return None;
        }
        let component = component.as_os_str().to_os_string();
        if !ancestor.pop() {
            return None;
        }
        suffix.push(component);
    }
}

#[cfg(test)]
#[path = "path_tests.rs"]
mod tests;
