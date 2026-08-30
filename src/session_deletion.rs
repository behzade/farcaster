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
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn deletes_every_session_file_in_a_family() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let root = temp.path().join("root.jsonl");
        let child = temp.path().join("child.jsonl");
        fs::write(&root, "root")?;
        fs::write(&child, "child")?;

        assert!(delete_family(&[root.clone(), child.clone()])?.is_empty());

        assert!(!root.exists());
        assert!(!child.exists());
        assert!(fs::read_dir(temp.path())?.next().is_none());
        Ok(())
    }

    #[test]
    fn restores_renamed_files_when_preparing_one_member_fails()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempdir()?;
        let root = temp.path().join("root.jsonl");
        let missing = temp.path().join("missing.jsonl");
        fs::write(&root, "root")?;

        let error = delete_family(&[root.clone(), missing])
            .expect_err("missing family member should prevent deletion");

        assert!(error.contains("prepare session deletion"));
        assert_eq!(fs::read_to_string(root)?, "root");
        assert_eq!(fs::read_dir(temp.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn attempts_every_quarantine_cleanup_and_reports_leftovers() {
        let quarantines = vec![
            (
                PathBuf::from("root.jsonl"),
                PathBuf::from("root.quarantine"),
            ),
            (
                PathBuf::from("child.jsonl"),
                PathBuf::from("child.quarantine"),
            ),
        ];
        let mut attempted = Vec::new();

        let leftovers = remove_quarantines(&quarantines, |path| {
            attempted.push(path.to_path_buf());
            if path == Path::new("root.quarantine") {
                Err(std::io::Error::other("busy"))
            } else {
                Ok(())
            }
        });

        assert_eq!(
            attempted,
            vec![
                PathBuf::from("root.quarantine"),
                PathBuf::from("child.quarantine")
            ]
        );
        assert_eq!(
            leftovers,
            vec![(PathBuf::from("root.quarantine"), "busy".to_owned())]
        );
    }
}
