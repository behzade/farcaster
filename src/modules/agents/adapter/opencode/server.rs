use std::{
    io::{BufRead as _, BufReader},
    process::{Child, ChildStdin, ChildStdout},
};

use serde::Deserialize;
use url::Url;

use super::{client::OpenCodeClient, event::OpenCodeEventStream, transport::OpenCodeTcpTransport};

pub(crate) struct OpenCodeServerProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    transport: OpenCodeTcpTransport,
}

impl OpenCodeServerProcess {
    pub(crate) fn attach(
        mut child: Child,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<Self, String> {
        let Some(stdin) = child.stdin.take() else {
            return attach_failed(child, None, "OpenCode server stdin must be piped".into());
        };
        let Some(stdout) = child.stdout.take() else {
            return attach_failed(
                child,
                Some(stdin),
                "OpenCode server stdout must be piped".into(),
            );
        };
        let endpoint = match read_endpoint(stdout) {
            Ok(endpoint) => endpoint,
            Err(error) => return attach_failed(child, Some(stdin), error),
        };
        let username = username.into();
        let password = password.into();
        let transport = match OpenCodeTcpTransport::new(endpoint, &username, &password) {
            Ok(transport) => transport,
            Err(error) => return attach_failed(child, Some(stdin), error),
        };
        Ok(Self {
            child,
            stdin: Some(stdin),
            transport,
        })
    }

    pub(crate) fn client(&self) -> OpenCodeClient<OpenCodeTcpTransport> {
        OpenCodeClient::new(self.transport.clone())
    }

    pub(crate) fn event_stream(&self) -> Result<OpenCodeEventStream, String> {
        OpenCodeEventStream::connect(&self.transport)
    }

    pub(crate) fn endpoint(&self) -> &Url {
        self.transport.endpoint()
    }

    pub(crate) fn terminate(&mut self) -> Result<(), String> {
        self.stdin.take();
        if self
            .child
            .try_wait()
            .map_err(|error| format!("check OpenCode server process: {error}"))?
            .is_none()
        {
            self.child
                .kill()
                .map_err(|error| format!("terminate OpenCode server process: {error}"))?;
        }
        self.child
            .wait()
            .map_err(|error| format!("reap OpenCode server process: {error}"))?;
        Ok(())
    }
}

impl Drop for OpenCodeServerProcess {
    fn drop(&mut self) {
        self.stdin.take();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn attach_failed<T>(
    mut child: Child,
    stdin: Option<ChildStdin>,
    error: String,
) -> Result<T, String> {
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    Err(error)
}

#[derive(Deserialize)]
struct ServerHandshake {
    url: String,
}

fn read_endpoint(stdout: ChildStdout) -> Result<Url, String> {
    let mut line = String::new();
    let read = BufReader::new(stdout)
        .read_line(&mut line)
        .map_err(|error| format!("read OpenCode server endpoint: {error}"))?;
    if read == 0 {
        return Err("OpenCode server closed before reporting its endpoint".into());
    }
    let handshake = serde_json::from_str::<ServerHandshake>(&line)
        .map_err(|error| format!("decode OpenCode server endpoint: {error}"))?;
    Url::parse(&handshake.url).map_err(|error| format!("parse OpenCode server endpoint: {error}"))
}

#[cfg(test)]
mod tests {
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
}
