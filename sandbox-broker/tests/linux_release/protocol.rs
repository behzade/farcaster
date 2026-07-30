use std::io::{BufReader, Read};
use std::process::Stdio;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use pi_sandbox_broker::framing::read_frame;
use pi_sandbox_broker::protocol::ServerEvent;

use super::support::{RELEASE_TEST_TIMEOUT, broker_command, write_invalid_empty_frame};

#[test]
#[ignore = "release gate: requires an unsandboxed Linux host with fixed Bubblewrap"]
fn linux_protocol_failure_release_gate() {
    let mut child = broker_command()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start broker for framing failure");
    let stdout = child.stdout.take().expect("broker stdout");
    let (ready_sender, ready_receiver) = mpsc::channel();
    let (trailing_sender, trailing_receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut output = BufReader::new(stdout);
        let ready = read_frame::<ServerEvent>(&mut output)
            .map_err(|error| error.to_string())
            .and_then(|event| event.ok_or_else(|| "broker closed before ready".to_owned()));
        if ready_sender.send(ready).is_err() {
            return;
        }
        let mut trailing = Vec::new();
        let result = output
            .read_to_end(&mut trailing)
            .map(|_| trailing)
            .map_err(|error| error.to_string());
        let _ = trailing_sender.send(result);
    });

    let ready = match ready_receiver.recv_timeout(RELEASE_TEST_TIMEOUT) {
        Ok(Ok(ready)) => ready,
        Ok(Err(message)) => {
            stop_broker(&mut child);
            panic!("broker readiness failed: {message}");
        }
        Err(error) => {
            stop_broker(&mut child);
            panic!("broker readiness timed out: {error}");
        }
    };
    if !matches!(ready, ServerEvent::Ready { can_exec: true, .. }) {
        stop_broker(&mut child);
        panic!("broker reported unavailable during framing gate: {ready:?}");
    }
    write_invalid_empty_frame(&mut child);

    let deadline = Instant::now() + RELEASE_TEST_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("wait for malformed-frame broker") {
            assert!(!status.success(), "malformed frame unexpectedly succeeded");
            let trailing = trailing_receiver
                .recv_timeout(RELEASE_TEST_TIMEOUT)
                .expect("broker output drain timed out")
                .expect("broker output drain failed");
            assert!(
                trailing.is_empty(),
                "malformed frame emitted a protocol response"
            );
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    stop_broker(&mut child);
    panic!("broker did not fail after a malformed frame");
}

fn stop_broker(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}
