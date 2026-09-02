use std::{
    collections::BTreeMap,
    io::{self, BufRead, BufReader, Read, Write},
    net::TcpStream,
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use url::{Host, Url};

use super::contract::{
    OpenCodeHttpMethod, OpenCodeHttpRequest, OpenCodeHttpResponse, OpenCodeHttpTransport,
};

#[derive(Clone)]
pub(crate) struct OpenCodeTcpTransport {
    endpoint: Url,
    authorization: String,
}

impl OpenCodeTcpTransport {
    pub(crate) fn new(endpoint: Url, username: &str, password: &str) -> Result<Self, String> {
        if endpoint.scheme() != "http" {
            return Err("OpenCode endpoint must use local HTTP".into());
        }
        let is_loopback = match endpoint.host() {
            Some(Host::Ipv4(address)) => address.is_loopback(),
            Some(Host::Ipv6(address)) => address.is_loopback(),
            Some(Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
            None => false,
        };
        if !is_loopback {
            return Err("OpenCode endpoint must use a loopback host".into());
        }
        let credentials = STANDARD.encode(format!("{username}:{password}"));
        Ok(Self {
            endpoint,
            authorization: format!("Basic {credentials}"),
        })
    }

    pub(crate) fn endpoint(&self) -> &Url {
        &self.endpoint
    }

    pub(super) fn open_event_stream(&self) -> Result<Box<dyn BufRead + Send>, String> {
        let request = OpenCodeHttpRequest {
            method: OpenCodeHttpMethod::Get,
            path: "/api/event".into(),
            body: None,
        };
        let mut reader = self.open(&request, "text/event-stream", "keep-alive")?;
        let (status, headers) = read_response_head(&mut reader)?;
        if !(200..300).contains(&status) {
            return Err(format!("OpenCode event stream returned HTTP {status}"));
        }
        reader
            .get_mut()
            .set_read_timeout(None)
            .map_err(|error| format!("configure OpenCode event stream: {error}"))?;
        if is_chunked(&headers) {
            Ok(Box::new(BufReader::new(ChunkedReader::new(reader))))
        } else {
            Ok(Box::new(reader))
        }
    }

    fn open(
        &self,
        request: &OpenCodeHttpRequest,
        accept: &str,
        connection: &str,
    ) -> Result<BufReader<TcpStream>, String> {
        let host = self
            .endpoint
            .host_str()
            .ok_or_else(|| "OpenCode endpoint has no host".to_owned())?;
        let port = self
            .endpoint
            .port_or_known_default()
            .ok_or_else(|| "OpenCode endpoint has no port".to_owned())?;
        let mut stream = TcpStream::connect((host, port))
            .map_err(|error| format!("connect to OpenCode server: {error}"))?;
        let timeout = Some(Duration::from_secs(15));
        stream
            .set_read_timeout(timeout)
            .and_then(|()| stream.set_write_timeout(timeout))
            .map_err(|error| format!("configure OpenCode connection timeout: {error}"))?;
        stream
            .write_all(&encode_request(
                host,
                port,
                &self.authorization,
                request,
                accept,
                connection,
            ))
            .and_then(|()| stream.flush())
            .map_err(|error| format!("write OpenCode request: {error}"))?;
        Ok(BufReader::new(stream))
    }
}

impl OpenCodeHttpTransport for OpenCodeTcpTransport {
    fn execute(&mut self, request: OpenCodeHttpRequest) -> Result<OpenCodeHttpResponse, String> {
        let reader = self.open(&request, "application/json", "close")?;
        decode_response(reader)
    }
}

fn encode_request(
    host: &str,
    port: u16,
    authorization: &str,
    request: &OpenCodeHttpRequest,
    accept: &str,
    connection: &str,
) -> Vec<u8> {
    let body = request.body.as_deref().unwrap_or_default();
    let mut encoded = format!(
        "{} {} HTTP/1.1\r\nHost: {host}:{port}\r\nAuthorization: {authorization}\r\nAccept: {accept}\r\nConnection: {connection}\r\nContent-Length: {}\r\n",
        method_name(request.method),
        request.path,
        body.len()
    )
    .into_bytes();
    if request.body.is_some() {
        encoded.extend_from_slice(b"Content-Type: application/json\r\n");
    }
    if let Some(directory) = request_directory(&request.path) {
        encoded.extend_from_slice(b"x-opencode-directory: ");
        encoded.extend_from_slice(directory.as_bytes());
        encoded.extend_from_slice(b"\r\n");
    }
    encoded.extend_from_slice(b"\r\n");
    encoded.extend_from_slice(body);
    encoded
}

fn request_directory(path: &str) -> Option<String> {
    let query = path.split_once('?')?.1;
    url::form_urlencoded::parse(query.as_bytes()).find_map(|(key, value)| {
        (key == "directory" && !value.bytes().any(|byte| matches!(byte, b'\r' | b'\n' | 0)))
            .then(|| value.into_owned())
    })
}

fn method_name(method: OpenCodeHttpMethod) -> &'static str {
    match method {
        OpenCodeHttpMethod::Get => "GET",
        OpenCodeHttpMethod::Post => "POST",
        OpenCodeHttpMethod::Patch => "PATCH",
        OpenCodeHttpMethod::Delete => "DELETE",
    }
}

fn decode_response(mut reader: impl BufRead) -> Result<OpenCodeHttpResponse, String> {
    let (status, headers) = read_response_head(&mut reader)?;
    if is_chunked(&headers) {
        let mut body = Vec::new();
        ChunkedReader::new(reader)
            .read_to_end(&mut body)
            .map_err(|error| format!("read OpenCode chunked response: {error}"))?;
        return Ok(OpenCodeHttpResponse { status, body });
    }
    let body = if let Some(length) = headers.get("content-length") {
        let length = length
            .parse::<usize>()
            .map_err(|error| format!("invalid OpenCode content length: {error}"))?;
        let mut body = vec![0; length];
        reader
            .read_exact(&mut body)
            .map_err(|error| format!("read OpenCode response body: {error}"))?;
        body
    } else {
        let mut body = Vec::new();
        reader
            .read_to_end(&mut body)
            .map_err(|error| format!("read OpenCode response body: {error}"))?;
        body
    };
    Ok(OpenCodeHttpResponse { status, body })
}

fn is_chunked(headers: &BTreeMap<String, String>) -> bool {
    headers.get("transfer-encoding").is_some_and(|value| {
        value
            .split(',')
            .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
    })
}

fn read_response_head(
    reader: &mut impl BufRead,
) -> Result<(u16, BTreeMap<String, String>), String> {
    let mut status_line = String::new();
    if reader
        .read_line(&mut status_line)
        .map_err(|error| format!("read OpenCode response status: {error}"))?
        == 0
    {
        return Err("OpenCode server closed before responding".into());
    }
    let status = status_line
        .split_ascii_whitespace()
        .nth(1)
        .ok_or_else(|| format!("invalid OpenCode response status: {}", status_line.trim()))?
        .parse::<u16>()
        .map_err(|error| format!("invalid OpenCode response status: {error}"))?;
    Ok((status, read_headers(reader)?))
}

fn read_headers(reader: &mut impl BufRead) -> Result<BTreeMap<String, String>, String> {
    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| format!("read OpenCode response headers: {error}"))?;
        if read == 0 {
            return Err("OpenCode server closed during response headers".into());
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            return Ok(headers);
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| format!("invalid OpenCode response header: {line}"))?;
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }
}

struct ChunkedReader<R> {
    inner: R,
    remaining: usize,
    done: bool,
}

impl<R> ChunkedReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            remaining: 0,
            done: false,
        }
    }
}

impl<R: BufRead> Read for ChunkedReader<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || self.done {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut size = String::new();
            if self.inner.read_line(&mut size)? == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "OpenCode chunked stream ended before a chunk size",
                ));
            }
            let size = size
                .trim_end_matches(['\r', '\n'])
                .split(';')
                .next()
                .unwrap_or_default();
            self.remaining = usize::from_str_radix(size, 16).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid OpenCode stream chunk size: {error}"),
                )
            })?;
            if self.remaining == 0 {
                read_headers(&mut self.inner).map_err(io::Error::other)?;
                self.done = true;
                return Ok(0);
            }
        }

        let read = self
            .inner
            .by_ref()
            .take(self.remaining as u64)
            .read(output)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "OpenCode chunked stream ended during a chunk",
            ));
        }
        self.remaining -= read;
        if self.remaining == 0 {
            let mut ending = [0; 2];
            self.inner.read_exact(&mut ending)?;
            if ending != *b"\r\n" {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid OpenCode stream chunk ending",
                ));
            }
        }
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn rejects_remote_endpoints_before_attaching_credentials() {
        let endpoint = Url::parse("http://example.com:4096").expect("valid URL");
        assert!(OpenCodeTcpTransport::new(endpoint, "opencode", "secret").is_err());
    }

    #[test]
    fn request_includes_local_auth_and_json_metadata() {
        let request = OpenCodeHttpRequest {
            method: OpenCodeHttpMethod::Post,
            path: "/api/session".into(),
            body: Some(br#"{"title":"Review"}"#.to_vec()),
        };
        let encoded = String::from_utf8(encode_request(
            "127.0.0.1",
            4096,
            "Basic secret",
            &request,
            "application/json",
            "close",
        ))
        .expect("HTTP request is UTF-8");
        assert!(encoded.starts_with("POST /api/session HTTP/1.1\r\n"));
        assert!(encoded.contains("Authorization: Basic secret\r\n"));
        assert!(encoded.contains("Content-Type: application/json\r\n"));
        assert!(encoded.ends_with(r#"{"title":"Review"}"#));
    }

    #[test]
    fn directory_queries_also_set_the_runtime_context_header() {
        let request = OpenCodeHttpRequest {
            method: OpenCodeHttpMethod::Get,
            path: "/api/model?directory=%2Ftmp%2Fproject+one".into(),
            body: None,
        };
        let encoded = String::from_utf8(encode_request(
            "127.0.0.1",
            4096,
            "Basic secret",
            &request,
            "application/json",
            "close",
        ))
        .expect("HTTP request is UTF-8");
        assert!(encoded.contains("x-opencode-directory: /tmp/project one\r\n"));
    }

    #[test]
    fn decodes_content_length_response() -> Result<(), String> {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\n\r\n{\"data\":true}";
        let decoded = decode_response(BufReader::new(Cursor::new(response)))?;
        assert_eq!(decoded.status, 200);
        assert_eq!(decoded.body, br#"{"data":true}"#);
        Ok(())
    }

    #[test]
    fn decodes_chunked_response_and_trailers() -> Result<(), String> {
        let response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n7\r\n{\"data\"\r\n6\r\n:true}\r\n0\r\nX-End: yes\r\n\r\n";
        let decoded = decode_response(BufReader::new(Cursor::new(response)))?;
        assert_eq!(decoded.body, br#"{"data":true}"#);
        Ok(())
    }
}
