use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{BufRead as _, BufReader, Write as _},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

use crate::sessions::{SessionTransfer, TransferMember};

pub(in crate::modules::agents::adapter) fn move_to_project(
    members: &[TransferMember],
    root_id: &str,
    project: &Path,
    source: &Path,
) -> Result<SessionTransfer, String> {
    let root = super::session_files::configured_session_root()?;
    let destination = destination_directory(&root, project, source);
    move_family(members, root_id, project, &destination)
}

pub(crate) fn destination_directory(
    session_root: &Path,
    project: &Path,
    source_session: &Path,
) -> PathBuf {
    if std::env::var_os("PI_CODING_AGENT_SESSION_DIR").is_some() {
        return source_session
            .parent()
            .unwrap_or(session_root)
            .to_path_buf();
    }
    let resolved = project
        .canonicalize()
        .unwrap_or_else(|_| project.to_path_buf());
    let encoded = resolved
        .to_string_lossy()
        .trim_start_matches(['/', '\\'])
        .replace(['/', '\\', ':'], "-");
    session_root.join(format!("--{encoded}--"))
}

pub(crate) fn move_family(
    members: &[TransferMember],
    root_id: &str,
    target_project: &Path,
    destination: &Path,
) -> Result<SessionTransfer, String> {
    let target_project = target_project.canonicalize().map_err(|error| {
        format!(
            "resolve target project {}: {error}",
            target_project.display()
        )
    })?;
    let (root, parents) = validate_family(members, root_id)?;
    fs::create_dir_all(destination).map_err(|error| {
        format!(
            "create target session directory {}: {error}",
            destination.display()
        )
    })?;

    let mut destinations = HashMap::new();
    for member in members {
        let file_name = member
            .path
            .file_name()
            .ok_or_else(|| format!("session path has no file name: {}", member.path.display()))?;
        let target = destination.join(file_name);
        if target.exists() && target != member.path {
            return Err(format!(
                "target session already exists: {}",
                target.display()
            ));
        }
        destinations.insert(member.path.clone(), target);
    }
    let by_id = members
        .iter()
        .map(|member| (member.id.as_str(), member))
        .collect::<HashMap<_, _>>();
    let nonce = transfer_nonce();
    let mut staged = Vec::new();
    for (index, member) in members.iter().enumerate() {
        let target = destinations
            .get(&member.path)
            .expect("every family member has a destination");
        let stage = destination.join(format!(".pi-move-{nonce}-{index}.tmp"));
        let parent = parents
            .get(member.id.as_str())
            .and_then(|parent| parent.as_deref())
            .map(|parent_id| {
                let parent = by_id
                    .get(parent_id)
                    .expect("validated parent belongs to family");
                destinations
                    .get(&parent.path)
                    .expect("parent has a destination")
                    .to_string_lossy()
                    .into_owned()
            });
        if let Err(error) = stage_session(
            &member.path,
            &stage,
            &member.id,
            &target_project,
            parent.as_deref(),
        ) {
            cleanup_files(staged.iter().map(|(_, stage)| stage));
            return Err(error);
        }
        staged.push((target.clone(), stage));
    }

    let quarantines = quarantine_sources(members, &nonce).inspect_err(|_| {
        cleanup_files(staged.iter().map(|(_, stage)| stage));
    })?;
    let mut committed = Vec::new();
    for (target, stage) in &staged {
        if let Err(error) = fs::rename(stage, target) {
            cleanup_files(committed.iter());
            restore_sources(&quarantines);
            cleanup_files(staged.iter().map(|(_, stage)| stage));
            return Err(format!(
                "commit moved session {}: {error}",
                target.display()
            ));
        }
        committed.push(target.clone());
    }

    for (_, quarantine) in &quarantines {
        let _ = fs::remove_file(quarantine);
    }
    Ok(SessionTransfer {
        root: destinations
            .get(&root.path)
            .expect("root has a destination")
            .clone(),
        paths: destinations,
    })
}

fn validate_family<'a>(
    members: &'a [TransferMember],
    root_id: &str,
) -> Result<(&'a TransferMember, HashMap<&'a str, Option<String>>), String> {
    let mut ids = HashSet::new();
    let mut paths = HashSet::new();
    for member in members {
        if !ids.insert(member.id.as_str()) {
            return Err(format!("duplicate session id in move: {}", member.id));
        }
        if !paths.insert(member.path.as_path()) {
            return Err(format!(
                "duplicate session path in move: {}",
                member.path.display()
            ));
        }
    }
    let root = members
        .iter()
        .find(|member| member.id == root_id)
        .ok_or_else(|| format!("session family does not contain root {root_id}"))?;
    if root.parent_id.is_some() {
        return Err("only a root session family can be moved".to_owned());
    }
    let mut parents = HashMap::new();
    for member in members {
        if member.id != root_id {
            let parent = member.parent_id.as_deref().ok_or_else(|| {
                format!("descendant session {} has no parent", member.path.display())
            })?;
            if !ids.contains(parent) {
                return Err(format!(
                    "descendant session {} has a parent outside the moved family",
                    member.path.display()
                ));
            }
        }
        parents.insert(member.id.as_str(), member.parent_id.clone());
    }
    Ok((root, parents))
}

fn stage_session(
    source: &Path,
    stage: &Path,
    expected_id: &str,
    target_project: &Path,
    parent: Option<&str>,
) -> Result<(), String> {
    let source_file = File::open(source)
        .map_err(|error| format!("open session {}: {error}", source.display()))?;
    let mut reader = BufReader::new(source_file);
    let mut header_line = Vec::new();
    if reader
        .read_until(b'\n', &mut header_line)
        .map_err(|error| format!("read session header {}: {error}", source.display()))?
        == 0
    {
        return Err(format!("session is empty: {}", source.display()));
    }
    let mut header: Value = serde_json::from_slice(&header_line)
        .map_err(|error| format!("decode session header {}: {error}", source.display()))?;
    let object = header
        .as_object_mut()
        .filter(|header| header.get("type").and_then(Value::as_str) == Some("session"))
        .ok_or_else(|| format!("invalid session header: {}", source.display()))?;
    if object.get("id").and_then(Value::as_str) != Some(expected_id) {
        return Err(format!(
            "session id changed before move: {}",
            source.display()
        ));
    }
    object.insert(
        "cwd".to_owned(),
        Value::String(target_project.to_string_lossy().into_owned()),
    );
    match parent {
        Some(parent) => {
            object.insert("parentSession".to_owned(), Value::String(parent.to_owned()));
        }
        None => {
            object.remove("parentSession");
        }
    }

    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(stage)
        .map_err(|error| format!("stage session {}: {error}", stage.display()))?;
    serde_json::to_writer(&mut output, &header)
        .map_err(|error| format!("encode session header {}: {error}", source.display()))?;
    output
        .write_all(b"\n")
        .and_then(|()| std::io::copy(&mut reader, &mut output).map(|_| ()))
        .and_then(|()| output.sync_all())
        .map_err(|error| format!("stage session {}: {error}", stage.display()))
}

fn quarantine_sources(
    members: &[TransferMember],
    nonce: &str,
) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let mut quarantines = Vec::new();
    for (index, member) in members.iter().enumerate() {
        let file_name = member
            .path
            .file_name()
            .ok_or_else(|| format!("session path has no file name: {}", member.path.display()))?;
        let quarantine = member.path.with_file_name(format!(
            ".{}.pi-move-{nonce}-{index}.quarantine",
            file_name.to_string_lossy()
        ));
        if let Err(error) = fs::rename(&member.path, &quarantine) {
            restore_sources(&quarantines);
            return Err(format!(
                "quarantine source session {}: {error}",
                member.path.display()
            ));
        }
        quarantines.push((member.path.clone(), quarantine));
    }
    Ok(quarantines)
}

fn restore_sources(quarantines: &[(PathBuf, PathBuf)]) {
    for (source, quarantine) in quarantines.iter().rev() {
        let _ = fs::rename(quarantine, source);
    }
}

fn cleanup_files<'a>(paths: impl IntoIterator<Item = &'a PathBuf>) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn transfer_nonce() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nanos}", std::process::id())
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
