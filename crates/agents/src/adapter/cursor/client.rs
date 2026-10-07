//! Cursor's sdk.v1 Connect/JSON protocol (cursor/sdk-bridge v1.0.35).
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

use serde_json::Value;

const MAX_MESSAGE: usize = 32 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct Client {
    address: SocketAddr,
    token: String,
}

impl Client {
    pub(super) fn new(url: &str, token: String) -> Result<Self, String> {
        let url = url::Url::parse(url).map_err(|e| e.to_string())?;
        let address = match url.host() {
            Some(url::Host::Ipv4(ip)) => std::net::IpAddr::V4(ip),
            Some(url::Host::Ipv6(ip)) => std::net::IpAddr::V6(ip),
            _ => return Err("Cursor SDK bridge must use a loopback IP address".into()),
        };
        if url.scheme() != "http" || !address.is_loopback() {
            return Err("Cursor SDK bridge must use loopback HTTP".into());
        }
        if token.is_empty() || token.bytes().any(|b| b.is_ascii_control()) {
            return Err("Invalid Cursor SDK bridge token".into());
        }
        Ok(Self {
            address: SocketAddr::new(address, url.port().ok_or("Bridge port missing")?),
            token,
        })
    }

    pub(super) fn call(&self, service: &str, method: &str, body: Value) -> Result<Value, String> {
        let mut timing = super::timing::Call::start(format!("{service}.{method}"));
        let result = (|| {
            let mut reader = self.open(service, method, body, false)?;
            let mut body = Vec::new();
            reader
                .by_ref()
                .take(MAX_MESSAGE as u64 + 1)
                .read_to_end(&mut body)
                .map_err(|e| format!("Read Cursor SDK {method}: {e}"))?;
            if body.len() > MAX_MESSAGE {
                return Err("Cursor SDK response too large".into());
            }
            serde_json::from_slice(&body).map_err(|e| format!("Decode Cursor SDK {method}: {e}"))
        })();
        timing.finish(result.is_ok());
        result
    }

    pub(super) fn send(&self, body: Value) -> Result<Stream, String> {
        let mut timing = super::timing::Call::start("SdkAgentService.Send");
        let reader = match self.open("SdkAgentService", "Send", body, true) {
            Ok(reader) => reader,
            Err(error) => {
                timing.finish(false);
                return Err(error);
            }
        };
        timing.record("headers");
        Ok(Stream {
            reader,
            ended: false,
            timing,
            first_event: false,
        })
    }

    fn open(
        &self,
        service: &str,
        method: &str,
        body: Value,
        streaming: bool,
    ) -> Result<Box<dyn Read + Send>, String> {
        let mut body = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
        if body.len() > MAX_MESSAGE {
            return Err("Cursor SDK request too large".into());
        }
        if streaming {
            body = frame(&body);
        }
        let mut socket = TcpStream::connect_timeout(&self.address, Duration::from_secs(5))
            .map_err(|e| format!("Connect Cursor SDK: {e}"))?;
        socket
            .set_read_timeout(Some(Duration::from_secs(if method == "Shutdown" {
                2
            } else if method == "CancelRun" {
                5
            } else {
                60
            })))
            .map_err(|e| e.to_string())?;
        socket
            .set_write_timeout(Some(Duration::from_secs(15)))
            .map_err(|e| e.to_string())?;
        let content_type = if streaming {
            "application/connect+json"
        } else {
            "application/json"
        };
        write!(socket, "POST /sdk.v1.{service}/{method} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: {content_type}\r\nConnect-Protocol-Version: 1\r\nConnection: close\r\nContent-Length: {}\r\n\r\n", self.address, self.token, body.len())
            .and_then(|()| socket.write_all(&body)).map_err(|e| format!("Write Cursor SDK {method}: {e}"))?;
        let mut reader = BufReader::new(socket);
        let status = line(&mut reader)?
            .split_ascii_whitespace()
            .nth(1)
            .ok_or("Cursor SDK HTTP status missing")?
            .parse::<u16>()
            .map_err(|e| e.to_string())?;
        let mut chunked = false;
        let mut length = None;
        let mut header_bytes = 0;
        loop {
            let header = line(&mut reader)?;
            header_bytes += header.len();
            if header_bytes > 65536 {
                return Err("Cursor SDK headers too large".into());
            }
            if header.is_empty() {
                break;
            }
            let (name, value) = header
                .split_once(':')
                .ok_or("Invalid Cursor SDK HTTP header")?;
            if name.eq_ignore_ascii_case("transfer-encoding") {
                chunked = value.trim().eq_ignore_ascii_case("chunked");
            }
            if name.eq_ignore_ascii_case("content-length") {
                length = Some(value.trim().parse::<u64>().map_err(|e| e.to_string())?);
            }
        }
        let mut reader: Box<dyn Read + Send> = if chunked {
            Box::new(Chunks {
                reader,
                remaining: 0,
                ended: false,
            })
        } else if let Some(length) = length {
            Box::new(reader.take(length))
        } else {
            Box::new(reader)
        };
        if !(200..300).contains(&status) {
            let mut body = String::new();
            reader
                .by_ref()
                .take(65536)
                .read_to_string(&mut body)
                .map_err(|e| e.to_string())?;
            let detail = serde_json::from_str::<Value>(&body)
                .ok()
                .map(|v| rpc_error(&v))
                .unwrap_or_else(|| format!("HTTP {status}"));
            return Err(format!("Cursor SDK {method}: {detail}"));
        }
        Ok(reader)
    }
}

pub(super) fn frame(body: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(body.len() + 5);
    result.push(0);
    result.extend_from_slice(&(body.len() as u32).to_be_bytes());
    result.extend_from_slice(body);
    result
}

pub(super) struct Stream {
    reader: Box<dyn Read + Send>,
    ended: bool,
    timing: super::timing::Call,
    first_event: bool,
}

impl Stream {
    pub(super) fn next(&mut self) -> Result<Option<Value>, String> {
        let result = self.read_next();
        match &result {
            Ok(None) => self.timing.finish(true),
            Err(_) => self.timing.finish(false),
            Ok(Some(value))
                if !self.first_event && value.as_object().is_some_and(|v| !v.is_empty()) =>
            {
                self.first_event = true;
                self.timing.record("first_event");
            }
            _ => {}
        }
        result
    }

    fn read_next(&mut self) -> Result<Option<Value>, String> {
        if self.ended {
            return Ok(None);
        }
        let mut header = [0; 5];
        self.reader
            .read_exact(&mut header)
            .map_err(|e| format!("Cursor SDK stream ended without a Connect trailer: {e}"))?;
        if !matches!(header[0], 0 | 2) {
            return Err("Unsupported Cursor SDK frame flags".into());
        }
        let length =
            u32::from_be_bytes(header[1..].try_into().expect("four length bytes")) as usize;
        if length > MAX_MESSAGE {
            return Err("Cursor SDK frame too large".into());
        }
        let mut body = vec![0; length];
        self.reader
            .read_exact(&mut body)
            .map_err(|e| format!("Read Cursor SDK frame: {e}"))?;
        let value: Value =
            serde_json::from_slice(&body).map_err(|e| format!("Decode Cursor SDK frame: {e}"))?;
        if header[0] == 2 {
            self.ended = true;
            if let Some(error) = value.get("error") {
                return Err(rpc_error(error));
            }
            return Ok(None);
        }
        Ok(Some(value))
    }
}

fn rpc_error(value: &Value) -> String {
    format!(
        "{}: {}",
        value["code"].as_str().unwrap_or("unknown"),
        value["message"]
            .as_str()
            .unwrap_or("Cursor SDK request failed")
    )
}

fn line(reader: &mut impl BufRead) -> Result<String, String> {
    let mut line = String::new();
    if reader
        .take(65537)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?
        == 0
    {
        return Err("Cursor SDK HTTP connection closed".into());
    }
    if line.len() > 65536 || !line.ends_with("\r\n") {
        return Err("Invalid Cursor SDK HTTP line".into());
    }
    line.truncate(line.len() - 2);
    Ok(line)
}

struct Chunks<R> {
    reader: R,
    remaining: usize,
    ended: bool,
}

impl<R: BufRead> Read for Chunks<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        use std::io::{Error, ErrorKind};
        if self.ended || buffer.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let size = line(&mut self.reader).map_err(Error::other)?;
            self.remaining = usize::from_str_radix(size.split(';').next().unwrap_or_default(), 16)
                .map_err(Error::other)?;
            if self.remaining == 0 {
                let mut bytes = 0;
                loop {
                    let trailer = line(&mut self.reader).map_err(Error::other)?;
                    bytes += trailer.len();
                    if bytes > 65536 {
                        return Err(Error::other("HTTP trailers too large"));
                    }
                    if trailer.is_empty() {
                        break;
                    }
                }
                self.ended = true;
                return Ok(0);
            }
        }
        let count = self.remaining.min(buffer.len());
        let read = self.reader.read(&mut buffer[..count])?;
        if read == 0 {
            return Err(Error::new(ErrorKind::UnexpectedEof, "Truncated HTTP chunk"));
        }
        self.remaining -= read;
        if self.remaining == 0 {
            let mut crlf = [0; 2];
            self.reader.read_exact(&mut crlf)?;
            if crlf != *b"\r\n" {
                return Err(Error::other("Invalid HTTP chunk ending"));
            }
        }
        Ok(read)
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
