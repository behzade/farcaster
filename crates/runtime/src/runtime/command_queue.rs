use std::sync::mpsc::{Receiver, TryRecvError};

use super::RuntimeCommand;

pub(super) fn receive_command(
    receiver: &Receiver<RuntimeCommand>,
    pending: &mut Option<RuntimeCommand>,
) -> Result<RuntimeCommand, TryRecvError> {
    let mut command = match pending.take() {
        Some(command) => command,
        None => receiver.try_recv()?,
    };
    if !matches!(
        command,
        RuntimeCommand::LoadSessions(_) | RuntimeCommand::SystemWake
    ) {
        return Ok(command);
    }
    loop {
        match (&mut command, receiver.try_recv()) {
            (RuntimeCommand::LoadSessions(query), Ok(RuntimeCommand::LoadSessions(next))) => {
                *query = next
            }
            (RuntimeCommand::SystemWake, Ok(RuntimeCommand::SystemWake)) => {}
            (_, Ok(next)) => {
                *pending = Some(next);
                break;
            }
            (_, Err(_)) => break,
        }
    }
    Ok(command)
}

#[cfg(test)]
#[path = "command_queue_tests.rs"]
mod tests;
