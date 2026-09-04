use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, SystemTime},
};

pub(crate) fn nearest_decision(
    path: &Path,
    project: &Path,
) -> Result<Option<(PathBuf, bool)>, String> {
    let _lock = TrustFileLock::acquire(path)?;
    let data = read_trust_file(path)?;
    let project = canonical(project)?;
    let mut current = Some(project.as_path());
    while let Some(directory) = current {
        let key = directory.display().to_string();
        if let Some(Some(decision)) = data.get(&key) {
            return Ok(Some((directory.to_path_buf(), *decision)));
        }
        current = directory.parent();
    }
    Ok(None)
}

pub(crate) fn update_trust_file(
    path: &Path,
    updates: &[(PathBuf, Option<bool>)],
) -> Result<(), String> {
    let _lock = TrustFileLock::acquire(path)?;
    let mut data = read_trust_file(path)?;
    for (path, decision) in updates {
        let key = canonical(path)?.display().to_string();
        if let Some(decision) = decision {
            data.insert(key, Some(*decision));
        } else {
            data.remove(&key);
        }
    }
    write_trust_file(path, &data)
}

fn read_trust_file(path: &Path) -> Result<BTreeMap<String, Option<bool>>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(format!("read trust store {}: {error}", path.display())),
    };
    let value = serde_json::from_slice::<serde_json::Value>(&bytes)
        .map_err(|error| format!("decode trust store {}: {error}", path.display()))?;
    let object = value
        .as_object()
        .ok_or_else(|| format!("invalid trust store {}: expected an object", path.display()))?;
    let mut data = BTreeMap::new();
    for (key, value) in object {
        let decision = match value {
            serde_json::Value::Bool(decision) => Some(*decision),
            serde_json::Value::Null => None,
            _ => {
                return Err(format!(
                    "invalid trust store {}: value for {key:?} must be true, false, or null",
                    path.display()
                ));
            }
        };
        data.insert(key.clone(), decision);
    }
    Ok(data)
}

fn write_trust_file(path: &Path, data: &BTreeMap<String, Option<bool>>) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("trust store has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).map_err(|error| format!("create {}: {error}", parent.display()))?;
    let mut bytes = serde_json::to_vec_pretty(data)
        .map_err(|error| format!("encode trust store {}: {error}", path.display()))?;
    bytes.push(b'\n');
    fs::write(path, bytes).map_err(|error| format!("write {}: {error}", path.display()))
}

pub(crate) fn canonical(path: &Path) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|error| format!("resolve {}: {error}", path.display()))
}

struct TrustFileLock {
    path: PathBuf,
}

impl TrustFileLock {
    fn acquire(trust_path: &Path) -> Result<Self, String> {
        let parent = trust_path
            .parent()
            .ok_or_else(|| format!("trust store has no parent: {}", trust_path.display()))?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
        let path = trust_path.with_extension("json.lock");
        for attempt in 0..10 {
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && attempt < 9 => {
                    let stale = fs::metadata(&path)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                        .is_some_and(|age| age >= Duration::from_secs(10));
                    if stale {
                        let _ = fs::remove_dir_all(&path);
                    } else {
                        thread::sleep(Duration::from_millis(20));
                    }
                }
                Err(error) => return Err(format!("lock trust store {}: {error}", path.display())),
            }
        }
        Err(format!("lock trust store {}: timed out", path.display()))
    }
}

impl Drop for TrustFileLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.path);
    }
}
