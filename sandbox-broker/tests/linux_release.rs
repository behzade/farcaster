#![cfg(target_os = "linux")]

#[path = "linux_release/filesystem.rs"]
mod filesystem;
#[path = "linux_release/lifecycle.rs"]
mod lifecycle;
#[path = "linux_release/protocol.rs"]
mod protocol;
#[path = "linux_release/runtime.rs"]
mod runtime;
#[path = "linux_release/support.rs"]
mod support;

use std::net::TcpStream;
use std::os::unix::net::UnixStream;

#[test]
#[ignore = "release fixture: invoked inside the Linux sandbox gate"]
fn sandbox_probe_entrypoint() {
    let Ok(probe) = std::env::var("PI_SANDBOX_RELEASE_PROBE") else {
        return;
    };
    match probe.as_str() {
        "network" => {
            let error = TcpStream::connect("127.0.0.1:9")
                .expect_err("IP socket unexpectedly escaped the network seccomp filter");
            assert_eq!(error.raw_os_error(), Some(libc::EPERM));
            UnixStream::pair().expect("AF_UNIX socket pairs must remain usable");
        }
        "unix-socket" => {
            let path = std::env::var("PI_SANDBOX_RELEASE_SOCKET").expect("socket probe path");
            let error = UnixStream::connect(path)
                .expect_err("host Unix socket unexpectedly escaped the seccomp filter");
            assert_eq!(error.raw_os_error(), Some(libc::EPERM));
        }
        "output" => {
            use std::io::Write as _;
            std::io::stdout()
                .write_all(&vec![b'x'; 1024 * 1024])
                .expect("write output probe");
        }
        "namespaces" => {
            for name in ["user", "pid", "net", "ipc", "uts", "mnt"] {
                let expected =
                    std::env::var(format!("PI_SANDBOX_HOST_NS_{}", name.to_ascii_uppercase()))
                        .expect("host namespace identity");
                let actual = std::fs::read_link(format!("/proc/self/ns/{name}"))
                    .expect("sandbox namespace identity")
                    .to_string_lossy()
                    .into_owned();
                assert_ne!(actual, expected, "{name} namespace was not isolated");
            }
        }
        other => panic!("unknown sandbox release probe: {other}"),
    }
}
