use super::*;

#[cfg(unix)]
use std::{fs, os::unix::fs::symlink};

#[cfg(unix)]
#[test]
fn normalizes_a_nonexistent_child_through_a_symlinked_ancestor() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let alias = temp.path().join("project-alias");
    fs::create_dir(&project).unwrap();
    symlink(&project, &alias).unwrap();

    assert_eq!(
        normalize_session_path(&alias.join("synthetic/../future.jsonl")),
        project.canonicalize().unwrap().join("future.jsonl")
    );
}

#[cfg(unix)]
#[test]
fn preserves_a_symlink_then_parent_component_when_the_path_exists() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let actual = root.join("actual");
    let nested = actual.join("nested");
    let alias = root.join("alias");
    fs::create_dir_all(&nested).unwrap();
    fs::write(actual.join("marker.jsonl"), "{}").unwrap();
    symlink(&nested, &alias).unwrap();

    assert_eq!(
        normalize_session_path(&alias.join("../marker.jsonl")),
        actual.join("marker.jsonl").canonicalize().unwrap()
    );
}

#[cfg(unix)]
#[test]
fn preserves_a_symlink_then_parent_component_when_the_child_is_missing() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    let actual = root.join("actual");
    let nested = actual.join("nested");
    let alias = root.join("alias");
    fs::create_dir_all(&nested).unwrap();
    symlink(&nested, &alias).unwrap();

    assert_eq!(
        normalize_session_path(&alias.join("../future.jsonl")),
        actual.canonicalize().unwrap().join("future.jsonl")
    );
}

#[test]
fn keeps_a_nonexistent_relative_path_relative() {
    assert_eq!(
        normalize_session_path(Path::new("synthetic/../future.jsonl")),
        PathBuf::from("future.jsonl")
    );
}
