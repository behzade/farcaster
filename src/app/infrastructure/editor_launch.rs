use std::{
    ffi::{OsStr, OsString},
    io::Write as _,
    os::unix::{ffi::OsStrExt as _, fs::OpenOptionsExt as _},
    path::Path,
};

pub(crate) fn prepare(
    path: &Path,
    program: &Path,
    arguments: &[OsString],
    project: &Path,
) -> Result<String, String> {
    let environment = crate::agents::project_shell_environment(project)?
        .unwrap_or_else(|| std::env::vars_os().collect());
    write_launch(path, program, arguments, project, &environment)
}

fn write_launch(
    path: &Path,
    program: &Path,
    arguments: &[OsString],
    project: &Path,
    environment: &[(OsString, OsString)],
) -> Result<String, String> {
    if path.as_os_str().as_bytes().contains(&0) {
        return Err("editor launch path contains a NUL byte".to_owned());
    }
    let mut command = b"/usr/bin/env -i /bin/sh ".to_vec();
    quote(&mut command, path.as_os_str());
    let command =
        String::from_utf8(command).map_err(|_| "editor launch path is not UTF-8".to_owned())?;
    if program.as_os_str().is_empty() {
        return Err("editor program is empty".to_owned());
    }
    let mut script = b"/bin/rm -- \"$0\" || exit\ncd -- ".to_vec();
    quote(&mut script, project.as_os_str());
    script.extend_from_slice(b" || exit\nexec /usr/bin/env -i --");
    for (key, value) in environment {
        if key.is_empty() || key.as_bytes().contains(&b'=') {
            return Err("invalid editor environment name".to_owned());
        }
        if matches!(key.to_str(), Some("TERM" | "COLORTERM" | "TERM_PROGRAM")) {
            continue;
        }
        let mut assignment = key.clone();
        assignment.push("=");
        assignment.push(value);
        script.push(b' ');
        quote(&mut script, &assignment);
    }
    script
        .extend_from_slice(b" TERM=xterm-256color COLORTERM=truecolor TERM_PROGRAM=gpui-ghostty ");
    quote(&mut script, program.as_os_str());
    for argument in arguments {
        script.push(b' ');
        quote(&mut script, argument);
    }
    script.push(b'\n');
    if script.contains(&0) {
        return Err("editor launch contains a NUL byte".to_owned());
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| format!("create editor launch file: {error}"))?;
    file.write_all(&script)
        .map_err(|error| format!("write editor launch file: {error}"))?;
    Ok(command)
}

fn quote(output: &mut Vec<u8>, value: &OsStr) {
    output.push(b'\'');
    for byte in value.as_bytes() {
        if *byte == b'\'' {
            output.extend_from_slice(b"'\\''");
        } else {
            output.push(*byte);
        }
    }
    output.push(b'\'');
}

#[cfg(test)]
#[path = "editor_launch_tests.rs"]
mod tests;
