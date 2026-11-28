use super::*;

#[test]
fn image_http_fetches_bytes_and_preserves_response_status() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        use std::io::Write as _;
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 4096];
        stream.read(&mut request).unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 8\r\nConnection: close\r\n\r\n\x89PNG\r\n\x1a\n").unwrap();
    });
    let response = futures::executor::block_on(
        ImageHttpClient.send(
            Request::builder()
                .uri(format!("http://{address}/image.png"))
                .body(().into())
                .unwrap(),
        ),
    )
    .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "image/png");
    let mut bytes = Vec::new();
    futures::executor::block_on(futures::io::AsyncReadExt::read_to_end(
        &mut response.into_body(),
        &mut bytes,
    ))
    .unwrap();
    assert_eq!(bytes, b"\x89PNG\r\n\x1a\n");
    server.join().unwrap();
}
