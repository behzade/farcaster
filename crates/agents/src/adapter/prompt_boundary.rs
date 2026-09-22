//! A session-scoped, authenticated rendezvous for blocking native hooks.
//! Hooks carry no prompt text. The session owner alone selects and dispatches input.
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

pub(super) struct Boundary {
    pub session_id: String,
    stream: TcpStream,
}

impl Boundary {
    pub fn release(mut self, stop: bool) {
        let body = if stop {
            r#"{"continue":false,"stopReason":"Pending user input"}"#
        } else {
            "{}"
        };
        let _ = write!(
            self.stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    }
}

pub(super) struct PromptBoundary {
    pub url: String,
    incoming: mpsc::Receiver<Boundary>,
    enabled: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl PromptBoundary {
    pub fn new(wake: Option<thread::Thread>) -> Result<Self, String> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|e| format!("Start prompt boundary: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let token = uuid::Uuid::new_v4().to_string();
        let path = format!("/{token}");
        let url = format!(
            "http://{}{path}",
            listener.local_addr().map_err(|e| e.to_string())?
        );
        let (tx, incoming) = mpsc::sync_channel(32);
        let enabled = Arc::new(AtomicBool::new(false));
        let closed = Arc::new(AtomicBool::new(false));
        let active = enabled.clone();
        let closing = closed.clone();
        let thread = thread::Builder::new()
            .name("prompt-boundary".into())
            .spawn(move || {
                while !closing.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if let Some(boundary) = read_boundary(stream, &path) {
                                if !active.load(Ordering::Acquire) {
                                    boundary.release(false);
                                } else {
                                    match tx.try_send(boundary) {
                                        Ok(()) => {
                                            if let Some(wake) = &wake {
                                                wake.unpark();
                                            }
                                        }
                                        Err(
                                            mpsc::TrySendError::Full(boundary)
                                            | mpsc::TrySendError::Disconnected(boundary),
                                        ) => boundary.release(false),
                                    }
                                }
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::park_timeout(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            url,
            incoming,
            enabled,
            closed,
            thread: Some(thread),
        })
    }

    pub fn enable(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
    }
    pub fn poll(&self) -> Option<Boundary> {
        self.incoming.try_recv().ok()
    }
}

impl Drop for PromptBoundary {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
        while let Ok(boundary) = self.incoming.try_recv() {
            boundary.release(false);
        }
    }
}

fn read_boundary(mut stream: TcpStream, path: &str) -> Option<Boundary> {
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .ok()?;
    let mut reader = BufReader::new(&mut stream);
    let mut line = String::new();
    reader.by_ref().take(4096).read_line(&mut line).ok()?;
    if line.trim_end() != format!("POST {path} HTTP/1.1") {
        return None;
    }
    let mut length = None;
    let mut header_bytes = line.len();
    loop {
        line.clear();
        let read = reader.by_ref().take(4096).read_line(&mut line).ok()?;
        header_bytes += read;
        if read == 0 || header_bytes > 16_384 {
            return None;
        }
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let length = length.filter(|length| *length <= 1_048_576)?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    let body: serde_json::Value = serde_json::from_slice(&body).ok()?;
    let session_id = body.get("session_id")?.as_str()?.to_owned();
    Some(Boundary { session_id, stream })
}

#[cfg(test)]
#[path = "prompt_boundary_tests.rs"]
pub(super) mod tests;
