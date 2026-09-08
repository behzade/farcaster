use std::process::{Command, Stdio};

use super::*;

#[test]
fn attaches_to_stdio_handshake_and_owns_child_lifetime() -> Result<(), String> {
    let mut command = Command::new("sh");
    command
        .args(["-c", "printf '{\"url\":\"http://127.0.0.1:4096\"}\\n'; cat"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = command
        .spawn()
        .map_err(|error| format!("spawn fake OpenCode server: {error}"))?;

    let mut server = OpenCodeServerProcess::attach(child, "opencode", "test-password")?;
    assert_eq!(server.endpoint().as_str(), "http://127.0.0.1:4096/");
    let _client = server.client();
    server.terminate()?;
    Ok(())
}

#[test]
fn handshake_wait_is_bounded() -> Result<(), String> {
    let mut child = Command::new("sh")
        .args(["-c", "cat"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| format!("spawn silent server: {error}"))?;
    let stdout = child.stdout.take().ok_or("missing test stdout")?;

    let error = read_endpoint_with_timeout(&mut child, stdout, Duration::from_millis(20))
        .expect_err("silent server should time out");
    assert_eq!(error, "timed out waiting for OpenCode server endpoint");
    let _ = child.wait();
    Ok(())
}
