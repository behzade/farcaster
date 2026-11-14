use super::*;

#[test]
fn preview_streams_large_assets_without_an_image_preview_limit() {
    let directory = tempfile::tempdir().unwrap();
    let asset = directory.path().join("large.png");
    let size = 33 * 1024 * 1024;
    std::fs::File::create(&asset)
        .unwrap()
        .set_len(size)
        .unwrap();
    let url = url(&asset).unwrap();
    let mut stream = TcpStream::connect((url.host_str().unwrap(), url.port().unwrap())).unwrap();
    write!(
        stream,
        "GET {} HTTP/1.1\r\nHost: localhost\r\n\r\n",
        url.path()
    )
    .unwrap();
    let mut response = std::io::BufReader::new(stream);
    let mut header = String::new();
    loop {
        use std::io::BufRead as _;
        let mut line = String::new();
        response.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        assert!(!line.is_empty());
        header.push_str(&line);
    }
    assert!(header.starts_with("HTTP/1.1 200"), "{header}");
    assert!(header.contains(&format!("Content-Length: {size}")));
    assert_eq!(
        std::io::copy(&mut response, &mut std::io::sink()).unwrap(),
        size
    );
}

#[test]
fn preview_serves_relative_modules_and_encoded_names() {
    let directory = tempfile::tempdir().unwrap();
    let page = directory.path().join("slides ü.html");
    std::fs::write(&page, "<script type=module src=./slides.js></script>").unwrap();
    std::fs::write(
        directory.path().join("slides.js"),
        "export const slide = 1;",
    )
    .unwrap();
    let page_url = url(&page).unwrap();
    for (url, expected, mime) in [
        (page_url.clone(), "<script type=module", "text/html"),
        (
            page_url.join("slides.js").unwrap(),
            "export const slide",
            "text/javascript",
        ),
    ] {
        let mut stream =
            TcpStream::connect((url.host_str().unwrap(), url.port().unwrap())).unwrap();
        write!(
            stream,
            "GET {} HTTP/1.1\r\nHost: localhost\r\n\r\n",
            url.path()
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains(expected));
        assert!(response.contains(mime));
    }
}

#[test]
fn preview_rejects_paths_outside_its_root_and_hidden_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("public");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(directory.path().join("secret.txt"), "secret").unwrap();
    std::fs::write(root.join(".env"), "secret").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(directory.path().join("secret.txt"), root.join("escape.txt"))
        .unwrap();
    for path in [
        "../secret.txt",
        "%2e%2e/secret.txt",
        ".env",
        "escape.txt",
        "missing.png",
    ] {
        assert_eq!(asset_path(&root, path), None, "{path}");
    }
}

#[test]
fn preview_cache_reuses_roots_and_evicts_oldest_without_a_restart() {
    let mut roots = HashMap::new();
    let token = grant_root(&mut roots, PathBuf::from("/same"));
    for _ in 0..130 {
        assert_eq!(grant_root(&mut roots, PathBuf::from("/same")), token);
    }
    assert_eq!(roots.len(), 1);
    roots.get_mut(&token).unwrap().1 = Instant::now() - Duration::from_secs(60);
    for index in 0..128 {
        grant_root(&mut roots, PathBuf::from(format!("/preview-{index}")));
    }
    assert_eq!(roots.len(), 128);
    assert!(
        !roots.contains_key(&token),
        "oldest preview makes room for the new one"
    );
}
