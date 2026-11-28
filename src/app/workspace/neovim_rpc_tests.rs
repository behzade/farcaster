use std::os::unix::net::UnixListener;

use super::*;

const TEST_TIMEOUT: Duration = Duration::from_secs(3);

fn read_request(stream: &mut UnixStream, expected_method: &str) -> (Value, Vec<Value>) {
    let message = read_value_with_max_depth(stream, 32).expect("read RPC request");
    let Value::Array(fields) = message else {
        panic!("expected RPC request array");
    };
    let [kind, id, method, params] = <[Value; 4]>::try_from(fields).expect("request fields");
    assert_eq!(kind.as_u64(), Some(0));
    assert_eq!(method.as_str(), Some(expected_method));
    match params {
        Value::Array(params) => (id, params),
        _ => panic!("expected RPC params array"),
    }
}

fn reply(stream: &mut UnixStream, id: Value, result: Value) {
    write_value(
        stream,
        &Value::Array(vec![1.into(), id, Value::Nil, result]),
    )
    .expect("send RPC response");
}

fn accept_connection(listener: &UnixListener, channel: u64) -> UnixStream {
    let (mut stream, _) = listener.accept().expect("accept RPC connection");
    stream
        .set_read_timeout(Some(TEST_TIMEOUT))
        .expect("set peer timeout");
    let (id, params) = read_request(&mut stream, "nvim_get_api_info");
    assert!(params.is_empty());
    reply(
        &mut stream,
        id,
        Value::Array(vec![channel.into(), Value::Map(vec![])]),
    );
    let (id, params) = read_request(&mut stream, "nvim_set_var");
    assert_eq!(params, vec!["farcaster_rpc_channel".into(), channel.into()]);
    reply(&mut stream, id, Value::Nil);
    stream
}

#[test]
fn waits_for_startup_then_reconnects_after_timeout_without_replay() {
    let dir = tempfile::tempdir().expect("socket directory");
    let socket = dir.path().join("nvim.sock");
    let (selections, _) = async_channel::unbounded();
    let mut rpc = NvimRpc::new(socket.clone(), selections);
    let peer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        let listener = UnixListener::bind(socket).expect("start RPC listener");
        let mut stream = accept_connection(&listener, 5);
        let (id, _) = read_request(&mut stream, "nvim_exec_lua");
        reply(&mut stream, id, 42.into());

        let (_id, params) = read_request(&mut stream, "nvim_exec_lua");
        assert_eq!(params[0].as_str(), Some("mutating work"));
        // Simulate a command which runs but gives no response before the deadline.
        assert_eq!(stream.read(&mut [0]).expect("RPC stream closes"), 0);
        let mut reconnected = accept_connection(&listener, 6);
        let (id, params) = read_request(&mut reconnected, "nvim_exec_lua");
        assert_eq!(params[0].as_str(), Some("return 43"));
        reply(&mut reconnected, id, 43.into());
        assert_eq!(
            reconnected.read(&mut [0]).expect("idle RPC stream closes"),
            0
        );
    });

    assert_eq!(
        rpc.exec_with_timeout("return 42", vec![], TEST_TIMEOUT),
        Ok(42.into())
    );
    assert_eq!(
        rpc.exec_with_timeout("mutating work", vec![], Duration::from_millis(300)),
        Err("Neovim remote request timed out".into())
    );
    assert_eq!(
        rpc.exec_with_timeout("return 43", vec![], TEST_TIMEOUT),
        Ok(43.into())
    );
    drop(rpc);
    peer.join().expect("peer completes");
}
