use super::*;

#[test]
fn abort_classifies_abandoned_refreshes_as_cancelled() -> TestResult {
    let project = tempdir()?;
    let command = queue_rpc_fixture(project.path())?;
    let mut rpc = PiRpcProcess::spawn(&command, project.path(), None)?;
    // Do not poll: even a response buffered by the old reader is abandoned on restart.
    let state = rpc.send_request(SessionCommand::LoadState)?;
    let usage = rpc.send_request(SessionCommand::LoadUsage)?;
    rpc.send_request(SessionCommand::Abort)?;
    let responses: Vec<_> = std::iter::from_fn(|| rpc.try_next())
        .filter_map(|event| match event {
            SessionEvent::Response(response) => Some(response),
            _ => None,
        })
        .collect();
    for (id, operation) in [
        (state, crate::SessionOperation::LoadState),
        (usage, crate::SessionOperation::LoadUsage),
    ] {
        let response = responses
            .iter()
            .find(|response| response.id.as_ref() == Some(&id))
            .ok_or("missing cancellation response")?;
        let error = response
            .result
            .as_ref()
            .expect_err("cancelled request must fail");
        assert_eq!(error.operation, operation);
        assert_eq!(error.kind, crate::SessionResponseErrorKind::Cancelled);
    }
    rpc.terminate()?;
    Ok(())
}
