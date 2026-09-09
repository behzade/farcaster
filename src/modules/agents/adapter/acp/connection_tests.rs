use super::*;

#[test]
fn frames_are_bounded_individually_before_parsing() {
    futures::executor::block_on(async {
        let mut reader = futures::io::Cursor::new(b"abc\nxyz\n");
        assert_eq!(
            read_frame(&mut reader, 4).await.unwrap(),
            Some("abc".into())
        );
        assert_eq!(
            read_frame(&mut reader, 4).await.unwrap(),
            Some("xyz".into())
        );
        assert_eq!(read_frame(&mut reader, 4).await.unwrap(), None);
        for bytes in [b"abcde".as_slice(), b"abcde\n".as_slice()] {
            let mut reader = BufReader::with_capacity(2, futures::io::Cursor::new(bytes));
            assert_eq!(
                read_frame(&mut reader, 4).await.unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    });
}

#[test]
fn frames_preserve_unicode_and_accept_crlf() {
    futures::executor::block_on(async {
        let mut reader =
            BufReader::with_capacity(1, futures::io::Cursor::new("سلام\r\nlast".as_bytes()));
        assert_eq!(
            read_frame(&mut reader, 32).await.unwrap(),
            Some("سلام".into())
        );
        assert_eq!(
            read_frame(&mut reader, 32).await.unwrap(),
            Some("last".into())
        );
    });
}

#[cfg(unix)]
mod exchange {
    use super::*;
    use std::{
        io::{BufRead as _, Write as _},
        os::unix::net::UnixStream,
    };

    const PROFILE: AcpProfile = AcpProfile {
        backend: "example-acp",
        name: "Example ACP",
        command: "example",
        path_environment: "EXAMPLE_ACP_PATH",
        arguments: &["acp"],
        auth_method: Some("login"),
        force_argument: None,
        resume_method: "session/load",
        permission_modes: None,
    };

    struct Peer(std::io::BufReader<UnixStream>);

    impl Peer {
        fn read(&mut self) -> Value {
            let mut line = String::new();
            assert!(self.0.read_line(&mut line).unwrap() > 0);
            serde_json::from_str(&line).unwrap()
        }

        fn write(&mut self, value: Value) {
            writeln!(self.0.get_mut(), "{value}").unwrap();
            self.0.get_mut().flush().unwrap();
        }

        fn reply(&mut self, request: &Value, result: Value) {
            self.write(json!({"jsonrpc":"2.0", "id":request["id"], "result":result}));
        }

        fn initialize(&mut self) {
            let request = self.read();
            assert_eq!(request["method"], "initialize");
            assert_eq!(request["params"]["clientCapabilities"]["terminal"], false);
            self.reply(
                &request,
                json!({
                    "protocolVersion":1, "agentCapabilities":{"loadSession":true},
                    "authMethods":[{"id":"login", "name":"Login"}]
                }),
            );
            let request = self.read();
            assert_eq!(request["method"], "authenticate");
            assert_eq!(request["params"]["methodId"], "login");
            self.reply(&request, json!({}));
        }
    }

    fn connect(run: impl FnOnce(Peer) + Send + 'static) -> (AcpConnection, thread::JoinHandle<()>) {
        let (client, peer) = UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        peer.set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let connection = AcpConnection::new(
            blocking::Unblock::new(client.try_clone().unwrap()),
            blocking::Unblock::new(client),
            None,
        )
        .unwrap();
        let peer = thread::spawn(move || run(Peer(std::io::BufReader::new(peer))));
        (connection, peer)
    }

    fn next(connection: &AcpConnection) -> Result<AcpInbound, String> {
        connection
            .incoming
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
    }

    #[test]
    fn sdk_authenticates_and_preserves_replay_before_load_response() {
        let (mut connection, peer) = connect(|mut peer| {
            peer.initialize();
            let request = peer.read();
            assert_eq!(request["method"], "session/load");
            peer.write(
                json!({"jsonrpc":"2.0", "method":"session/update", "params":{
                    "sessionId":"one", "update":{"sessionUpdate":"agent_message_chunk",
                    "content":{"type":"text", "text":"history"}, "cursorExtra":42}
                }}),
            );
            peer.reply(
                &request,
                json!({"sessionId":"one", "cursorExtra":"retained"}),
            );
            assert_eq!(peer.read()["method"], "session/cancel");
        });
        assert_eq!(
            connection.initialize(&PROFILE).unwrap()["protocolVersion"],
            1
        );
        let result = connection
            .request_blocking(
                "session/load",
                json!({"sessionId":"one", "cwd":"/project", "mcpServers":[]}),
            )
            .unwrap();
        assert_eq!(result["cursorExtra"], "retained");
        let replay = connection.drain_queued().unwrap();
        assert_eq!(replay.len(), 1);
        assert!(
            matches!(&replay[0], AcpInbound::Notification {params,..} if params["update"]["cursorExtra"] == 42)
        );
        connection
            .notify("session/cancel", json!({"sessionId":"one"}))
            .unwrap();
        peer.join().unwrap();
    }

    #[test]
    fn pending_permission_does_not_block_updates_and_native_ids_survive() {
        let (connection, peer) = connect(|mut peer| {
            peer.initialize();
            let prompt = peer.read();
            peer.write(json!({"jsonrpc":"2.0", "id":"approve-7", "method":"session/request_permission", "params":{
                "sessionId":"one", "toolCall":{"toolCallId":"tool-1", "title":"Edit"},
                "options":[{"optionId":"allow-once-7", "name":"Allow", "kind":"allow_once"}]
            }}));
            peer.write(json!({"jsonrpc":"2.0", "method":"cursor/task", "params":{"extra":true}}));
            let answer = peer.read();
            assert_eq!(answer["id"], "approve-7");
            assert_eq!(answer["result"]["outcome"]["optionId"], "allow-once-7");
            peer.reply(&prompt, json!({"stopReason":"end_turn"}));
        });
        connection.initialize(&PROFILE).unwrap();
        let prompt_id = connection
            .send_request("session/prompt", json!({"sessionId":"one", "prompt":[]}))
            .unwrap();
        let AcpInbound::AgentRequest { id, method, .. } = next(&connection).unwrap() else {
            panic!("expected permission");
        };
        assert_eq!(method, "session/request_permission");
        assert!(
            matches!(next(&connection).unwrap(), AcpInbound::Notification {method,..} if method == "cursor/task")
        );
        connection
            .respond(
                &id,
                json!({"outcome":{"outcome":"selected", "optionId":"allow-once-7"}}),
            )
            .unwrap();
        assert!(
            matches!(next(&connection).unwrap(), AcpInbound::Response {id,..} if id == prompt_id)
        );
        peer.join().unwrap();
    }

    #[test]
    fn sdk_routes_errors_and_reports_eof() {
        let (connection, peer) = connect(|mut peer| {
            peer.initialize();
            let request = peer.read();
            peer.write(json!({"jsonrpc":"2.0", "id":request["id"], "error":{"code":-32602, "message":"bad model", "data":{"model":"missing"}}}));
        });
        connection.initialize(&PROFILE).unwrap();
        let id = connection
            .send_request(
                "session/set_config_option",
                json!({"sessionId":"one", "configId":"model", "value":"missing"}),
            )
            .unwrap();
        assert!(
            matches!(next(&connection).unwrap(), AcpInbound::Error {id: response_id, message} if response_id == id && message.contains("bad model"))
        );
        assert!(next(&connection).is_err());
        peer.join().unwrap();
    }

    #[test]
    fn startup_updates_can_be_inspected_then_delivered_live() {
        let (mut connection, peer) = connect(|mut peer| {
            peer.initialize();
            let request = peer.read();
            peer.write(
                json!({"jsonrpc":"2.0", "method":"session/update", "params":{
                    "sessionId":"one", "update":{"sessionUpdate":"available_commands_update",
                    "availableCommands":[{"name":"review", "description":"Review changes"}]}
                }}),
            );
            peer.reply(&request, json!({"sessionId":"one"}));
            assert_eq!(peer.read()["method"], "session/cancel");
        });
        connection.initialize(&PROFILE).unwrap();
        connection
            .request_blocking("session/new", json!({"cwd":"/project", "mcpServers":[]}))
            .unwrap();
        let queued = connection.drain_queued().unwrap();
        assert_eq!(queued.len(), 1);
        assert!(super::super::super::translate::commands_from_update(&queued[0], "one").is_some());
        connection.restore_queued(queued);
        assert!(matches!(
            connection.poll(),
            Some(Ok(AcpInbound::Notification { .. }))
        ));
        assert!(connection.poll().is_none());
        connection
            .notify("session/cancel", json!({"sessionId":"one"}))
            .unwrap();
        peer.join().unwrap();
    }

    #[test]
    fn unanswered_request_fails_when_agent_exits() {
        let (connection, peer) = connect(|mut peer| {
            peer.initialize();
            let _ = peer.read();
        });
        connection.initialize(&PROFILE).unwrap();
        assert!(
            connection
                .request_blocking("session/new", json!({"cwd":"/project", "mcpServers":[]}))
                .is_err()
        );
        peer.join().unwrap();
    }
}
