use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) fn delete_family(paths: &[PathBuf]) -> Result<Vec<(PathBuf, String)>, String> {
    if paths.is_empty() {
        return Err("session family is empty".to_owned());
    }
    let nonce = deletion_nonce();
    let mut quarantines = Vec::with_capacity(paths.len());
    for (index, path) in paths.iter().enumerate() {
        let file_name = path
            .file_name()
            .ok_or_else(|| format!("session path has no file name: {}", path.display()))?;
        let quarantine = path.with_file_name(format!(
            ".{}.pi-delete-{nonce}-{index}.quarantine",
            file_name.to_string_lossy()
        ));
        if let Err(error) = fs::rename(path, &quarantine) {
            restore_sources(&quarantines);
            return Err(format!(
                "prepare session deletion {}: {error}",
                path.display()
            ));
        }
        quarantines.push((path.clone(), quarantine));
    }

    Ok(remove_quarantines(&quarantines, |path| {
        fs::remove_file(path)
    }))
}

fn remove_quarantines(
    quarantines: &[(PathBuf, PathBuf)],
    mut remove: impl FnMut(&Path) -> std::io::Result<()>,
) -> Vec<(PathBuf, String)> {
    let mut leftovers = Vec::new();
    for (_, quarantine) in quarantines {
        if let Err(error) = remove(quarantine) {
            leftovers.push((quarantine.clone(), error.to_string()));
        }
    }
    leftovers
}

fn restore_sources(quarantines: &[(PathBuf, PathBuf)]) {
    for (source, quarantine) in quarantines.iter().rev() {
        let _ = fs::rename(quarantine, source);
    }
}

fn deletion_nonce() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nanos}", std::process::id())
}

#[cfg(test)]
#[path = "deletion_tests.rs"]
mod tests;
