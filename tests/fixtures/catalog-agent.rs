use std::{
    io::{self, BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    time::Duration,
};

fn main() -> io::Result<()> {
    let mut executable = std::env::current_exe()?;
    let mut args = std::env::args().skip(1).collect::<Vec<_>>();
    // Runtime tests replace both the native bridge and its Node helper with
    // the same protocol peer. The helper itself has a separate integration test.
    if executable.file_name() == Some(std::ffi::OsStr::new("node"))
        && args.first().map(String::as_str) == Some("--input-type=module")
    {
        executable = executable.parent().unwrap().join("cursor-cli");
        args = args.into_iter().skip(4).collect();
    }
    std::fs::write(executable.with_extension("args"), args.join("\n"))?;
    if executable.file_name() == Some(std::ffi::OsStr::new("opencode"))
        && args == ["debug", "paths"]
    {
        return Ok(());
    }
    let mut control = UnixStream::connect(executable.parent().unwrap().join("control.sock"))?;
    control.set_read_timeout(Some(Duration::from_secs(20)))?;
    writeln!(
        control,
        "{}",
        executable.file_name().unwrap().to_string_lossy()
    )?;
    if std::env::var_os("FARCASTER_FIXTURE_REPORT_MODE").is_some() {
        writeln!(
            control,
            "{}",
            if std::env::args().any(|arg| arg == "--print") {
                "print"
            } else {
                "rpc"
            }
        )?;
    }
    if executable.file_name() == Some(std::ffi::OsStr::new("cursor-cli")) {
        return cursor_sdk(control, executable.parent().unwrap());
    }
    let replies = control.try_clone()?;
    let forwarder = std::thread::spawn(move || {
        let mut output = io::stdout().lock();
        for line in BufReader::new(replies).lines() {
            let Ok(line) = line else { break };
            if writeln!(output, "{line}")
                .and_then(|_| output.flush())
                .is_err()
            {
                break;
            }
        }
        std::process::exit(0);
    });
    if std::env::var_os("FARCASTER_FIXTURE_REPORT_MODE").is_some()
        && std::env::args().any(|arg| arg == "--print")
    {
        let _ = forwarder.join();
        return Ok(());
    }
    for line in io::stdin().lock().lines() {
        writeln!(control, "{}", line?)?;
    }
    Ok(())
}

fn cursor_sdk(mut control: UnixStream, root: &std::path::Path) -> io::Result<()> {
    use std::{io::Read, net::TcpListener};
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let token = root.join("sdk-token");
    std::fs::write(&token, "fixture-token")?;
    eprintln!(
        "cursor-sdk-bridge ready {{\"schemaVersion\":1,\"transport\":\"tcp\",\"protocol\":\"connect\",\"url\":\"http://{address}\",\"authTokenFile\":\"{}\"}}",
        token.display()
    );
    let mut replies = BufReader::new(control.try_clone()?);
    for socket in listener.incoming() {
        let mut socket = BufReader::new(socket?);
        let mut first = String::new();
        socket.read_line(&mut first)?;
        let method = first
            .split_whitespace()
            .nth(1)
            .unwrap()
            .rsplit('/')
            .next()
            .unwrap();
        let mut length = 0;
        loop {
            let mut header = String::new();
            socket.read_line(&mut header)?;
            if header == "\r\n" {
                break;
            }
            if let Some(value) = header.strip_prefix("Content-Length: ") {
                length = value.trim().parse::<usize>().unwrap();
            }
        }
        let mut body = vec![0; length];
        socket.read_exact(&mut body)?;
        if method == "Send" {
            writeln!(
                control,
                "{{\"method\":\"Send\",\"params\":{}}}",
                String::from_utf8_lossy(&body[5..])
            )?;
            write!(
                socket.get_mut(),
                "HTTP/1.1 200 OK\r\nContent-Type: application/connect+json\r\nConnection: close\r\n\r\n"
            )?;
            loop {
                let mut event = String::new();
                replies.read_line(&mut event)?;
                if event.is_empty() {
                    return Ok(());
                }
                let event = event.trim();
                let end = event == "{}";
                socket.get_mut().write_all(&[if end { 2 } else { 0 }])?;
                socket
                    .get_mut()
                    .write_all(&(event.len() as u32).to_be_bytes())?;
                socket.get_mut().write_all(event.as_bytes())?;
                if end {
                    break;
                }
            }
            continue;
        }
        let response = match method {
            "GetVersion" => r#"{"protocolVersion":"sdk.v1"}"#.to_owned(),
            "Shutdown" => "{}".to_owned(),
            _ => {
                writeln!(
                    control,
                    "{{\"method\":\"{method}\",\"params\":{}}}",
                    String::from_utf8_lossy(&body)
                )?;
                let mut response = String::new();
                replies.read_line(&mut response)?;
                response
            }
        };
        write!(
            socket.get_mut(),
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response
        )?;
        if method == "Shutdown" {
            break;
        }
    }
    Ok(())
}
