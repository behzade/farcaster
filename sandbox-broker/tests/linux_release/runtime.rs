use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::thread;

use pi_sandbox_broker::protocol::{ExecRequest, NetworkPolicy};

use super::support::{Broker, TempRoot, request};

const OUTPUT_LIMIT: u64 = 4 * 1024;

#[test]
#[ignore = "release gate: requires an unsandboxed Linux host with fixed Bubblewrap"]
fn linux_runtime_release_gate() {
    let root = TempRoot::new("runtime");
    let workspace = root.0.join("workspace");
    fs::create_dir_all(&workspace).expect("create workspace");
    let mut broker = Broker::start();

    let runtime = broker.exec(request(
        "runtime-boundary",
        &workspace,
        concat!(
            "found=; while read -r key value rest; do ",
            "[ \"$key\" = NoNewPrivs: ] && found=$value; done < /proc/self/status; ",
            "test \"$found\" = 1 && test \"$$\" -le 2 && test -r /etc/os-release && ",
            "test \"$ONLY\" = yes && test -z \"${PI_RELEASE_HOST_SENTINEL:-}\" && printf runtime-ok"
        )
        .to_owned(),
        vec![],
        vec![],
        None,
        OUTPUT_LIMIT,
    ));
    assert_eq!(runtime.code, Some(0));
    assert_eq!(runtime.output, b"runtime-ok");

    let mut output_request = probe_request("output-cap", &workspace, "output", None);
    output_request.policy.output_limit_bytes = 1024;
    let output = broker.exec(output_request);
    assert_eq!(output.code, Some(0));
    assert_eq!(output.output.len(), 1024);
    assert!(output.truncated);

    let namespaces = broker.exec(probe_request(
        "namespace-isolation",
        &workspace,
        "namespaces",
        None,
    ));
    assert_eq!(
        namespaces.code,
        Some(0),
        "namespace probe failed:\n{}",
        String::from_utf8_lossy(&namespaces.output)
    );

    let network = broker.exec(probe_request(
        "network-seccomp",
        &workspace,
        "network",
        None,
    ));
    assert_eq!(
        network.code,
        Some(0),
        "network probe failed:\n{}",
        String::from_utf8_lossy(&network.output)
    );

    let mut loopback = request(
		"loopback-allowed",
		&workspace,
		"python3 -c 'import socket; s=socket.socket(); s.bind((\"127.0.0.1\",0)); s.listen(); c=socket.create_connection(s.getsockname()); a,_=s.accept(); c.sendall(b\"ok\"); print(a.recv(2).decode(),end=\"\")'".to_owned(),
		vec![],
		vec![],
		Some(5_000),
		64 * 1024,
	);
    loopback.policy.network = NetworkPolicy::Loopback;
    let loopback = broker.exec(loopback);
    assert_eq!(
        loopback.code,
        Some(0),
        "loopback probe failed:\n{}",
        String::from_utf8_lossy(&loopback.output)
    );
    assert_eq!(loopback.output, b"ok");

    let socket_path = root.0.join("host.sock");
    let _listener = UnixListener::bind(&socket_path).expect("host Unix socket fixture");
    let unix_socket = broker.exec(probe_request(
        "unix-socket-seccomp",
        &workspace,
        "unix-socket",
        Some(&socket_path),
    ));
    assert_eq!(
        unix_socket.code,
        Some(0),
        "Unix socket probe failed:\n{}",
        String::from_utf8_lossy(&unix_socket.output)
    );

    let proxy_path = root.0.join("network-proxy.sock");
    let listener = UnixListener::bind(&proxy_path).expect("host proxy fixture");
    let proxy_thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept bridge connection");
        let mut input = [0_u8; 4];
        stream.read_exact(&mut input).expect("read bridge input");
        assert_eq!(&input, b"ping");
        stream.write_all(b"pong").expect("write bridge output");
    });
    let mut proxied = request(
		"network-proxy-bridge",
		&workspace,
		"python3 -c 'import socket; s=socket.create_connection((\"127.0.0.1\",31128)); s.sendall(b\"ping\"); print(s.recv(4).decode(), end=\"\")'".to_owned(),
		vec![],
		vec![],
		Some(5_000),
		64 * 1024,
	);
    proxied.policy.network = NetworkPolicy::Proxy {
        tcp_port: 40_000,
        unix_socket: proxy_path.to_string_lossy().into_owned(),
        allow_local_binding: false,
    };
    let proxied = broker.exec(proxied);
    assert_eq!(
        proxied.code,
        Some(0),
        "proxy bridge failed:\n{}",
        String::from_utf8_lossy(&proxied.output)
    );
    assert_eq!(proxied.output, b"pong");
    proxy_thread.join().expect("proxy fixture thread");

    let direct_socket_path = root.0.join("direct-proxy.sock");
    let _direct_listener = UnixListener::bind(&direct_socket_path).expect("direct proxy fixture");
    let mut direct_socket = probe_request(
        "network-proxy-direct-socket",
        &workspace,
        "unix-socket",
        Some(&direct_socket_path),
    );
    direct_socket.policy.network = NetworkPolicy::Proxy {
        tcp_port: 40_001,
        unix_socket: direct_socket_path.to_string_lossy().into_owned(),
        allow_local_binding: false,
    };
    let direct_socket = broker.exec(direct_socket);
    assert_eq!(
        direct_socket.code,
        Some(0),
        "proxied command reached the host Unix socket directly:\n{}",
        String::from_utf8_lossy(&direct_socket.output)
    );
}

fn probe_request(id: &str, workspace: &Path, probe: &str, socket: Option<&Path>) -> ExecRequest {
    let mut request = request(
        id,
        workspace,
        String::new(),
        vec![],
        vec![],
        Some(5_000),
        64 * 1024,
    );
    request.command.program = std::env::current_exe()
        .expect("release test executable")
        .canonicalize()
        .expect("canonical release test executable")
        .to_string_lossy()
        .into_owned();
    request.command.args = vec![
        "--ignored".to_owned(),
        "--exact".to_owned(),
        "sandbox_probe_entrypoint".to_owned(),
        "--nocapture".to_owned(),
    ];
    request
        .env
        .insert("PI_SANDBOX_RELEASE_PROBE".to_owned(), probe.to_owned());
    if probe == "namespaces" {
        for name in ["user", "pid", "net", "ipc", "uts", "mnt"] {
            let identity = std::fs::read_link(format!("/proc/self/ns/{name}"))
                .expect("host namespace identity")
                .to_string_lossy()
                .into_owned();
            request.env.insert(
                format!("PI_SANDBOX_HOST_NS_{}", name.to_ascii_uppercase()),
                identity,
            );
        }
    }
    if let Some(path) = socket {
        request.env.insert(
            "PI_SANDBOX_RELEASE_SOCKET".to_owned(),
            path.to_string_lossy().into_owned(),
        );
    }
    request
}
