use super::*;

struct CloseFailure;

impl SessionTransport for CloseFailure {
    fn send(&mut self, _command: SessionCommand) -> Result<String, String> {
        Ok(String::new())
    }

    fn respond(&mut self, _response: ExtensionUiResponse) -> Result<(), String> {
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        None
    }

    fn close(&mut self) -> Result<(), String> {
        Err("transport close failed".into())
    }
}

#[test]
fn session_loop_shutdown_returns_the_transport_close_failure() {
    let error = close_process(Some(Box::new(CloseFailure))).expect_err("close must fail");
    assert_eq!(error, "transport close failed");
}
