use super::*;

#[test]
fn resumed_transports_do_not_reuse_prompt_receipt_ids() -> TestResult {
    let (temp, command) = fake("deferred-session")?;
    let session = temp.path().join("fake-session.jsonl");
    fs::write(&session, "")?;
    let mut ids = std::collections::HashSet::new();
    for _ in 0..2 {
        let mut rpc = PiRpcProcess::spawn(&command, temp.path(), Some(&session))?;
        for message in ["first prompt", "second prompt"] {
            let id = prompt(&mut rpc, crate::protocol::PromptMode::Normal, message)?;
            wait_for_response(&mut rpc, &id)?;
            assert!(ids.insert(id), "a relaunched transport reused a receipt ID");
        }
        rpc.terminate()?;
    }
    Ok(())
}
