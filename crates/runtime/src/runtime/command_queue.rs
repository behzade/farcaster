use std::sync::mpsc::{Receiver, TryRecvError};

use super::RuntimeCommand;

pub(super) fn receive_command(
    receiver: &Receiver<RuntimeCommand>,
    pending: &mut Option<RuntimeCommand>,
) -> Result<RuntimeCommand, TryRecvError> {
    let command = match pending.take() {
        Some(command) => command,
        None => receiver.try_recv()?,
    };
    let RuntimeCommand::LoadSessions(mut query) = command else {
        return Ok(command);
    };
    loop {
        match receiver.try_recv() {
            Ok(RuntimeCommand::LoadSessions(next)) => query = next,
            Ok(command) => {
                *pending = Some(command);
                break;
            }
            Err(_) => break,
        }
    }
    Ok(RuntimeCommand::LoadSessions(query))
}

#[cfg(test)]
#[path = "command_queue_tests.rs"]
mod tests;
