use super::*;
use std::os::unix::{
    ffi::{OsStrExt as _, OsStringExt as _},
    fs::PermissionsExt as _,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn launch_replaces_inherited_environment_with_exact_snapshot() -> TestResult {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("launch.json");
    let environment = vec![
        ("PATH".into(), "/captured/project/bin".into()),
        (
            "FARCASTER_CAPTURED_PROJECT_PATH".into(),
            "/captured/project/bin".into(),
        ),
        ("EMPTY".into(), "".into()),
        (
            "QUOTED".into(),
            "it's $HOME\n`not a command` = value".into(),
        ),
        ("BYTES".into(), OsString::from_vec(vec![0xff, b'x'])),
    ];
    write_launch(
        &path,
        &Launch {
            program: "/usr/bin/env".into(),
            arguments: vec!["-0".into()],
            project: directory.path().into(),
            environment: environment.clone(),
        },
    )?;
    assert_eq!(
        std::fs::metadata(&path)?.permissions().mode() & 0o777,
        0o600
    );
    let mut child = take_command(&path)?;
    assert!(
        !path.exists(),
        "consume the private environment snapshot before launch"
    );
    assert_eq!(child.get_current_dir(), Some(directory.path()));
    let output = child.output()?;
    assert!(output.status.success());
    let mut actual: Vec<Vec<u8>> = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(Vec::from)
        .collect();
    let mut expected: Vec<Vec<u8>> = environment
        .iter()
        .map(|(key, value)| {
            let mut entry = key.as_bytes().to_vec();
            entry.push(b'=');
            entry.extend_from_slice(value.as_bytes());
            entry
        })
        .collect();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
    Ok(())
}

#[test]
fn launch_preserves_arguments_and_uses_captured_path() -> TestResult {
    let directory = tempfile::tempdir()?;
    std::os::unix::fs::symlink("/bin/sh", directory.path().join("editor"))?;
    let path = directory.path().join("launch.json");
    write_launch(
        &path,
        &Launch {
            program: "editor".into(),
            arguments: vec![
                "-c".into(),
                "printf '%s' \"$1\"".into(),
                "editor".into(),
                "a 'quoted' $argument\n".into(),
            ],
            project: directory.path().into(),
            environment: vec![("PATH".into(), directory.path().as_os_str().into())],
        },
    )?;
    let output = take_command(&path)?.output()?;
    assert!(output.status.success());
    assert_eq!(output.stdout, b"a 'quoted' $argument\n");
    Ok(())
}
