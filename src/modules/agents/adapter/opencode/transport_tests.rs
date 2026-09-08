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
