use std::io;
#[cfg(target_os = "macos")]
use std::path::Path;

use pi_sandbox_broker::denial_collector::DenialCollector;
use pi_sandbox_broker::executor::Runtime;
use pi_sandbox_broker::framing::read_frame;
use pi_sandbox_broker::protocol::{
    ClientRequest, ErrorCode, MAX_FRAME_BYTES, PROTOCOL_VERSION, ServerEvent,
};
use pi_sandbox_broker::seatbelt::HardPolicy;
#[cfg(target_os = "macos")]
use pi_sandbox_broker::seatbelt::{SANDBOX_EXEC, self_test};
use pi_sandbox_broker::validation::validate_exec;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stdin = io::stdin();
    let mut reader = stdin.lock();

    let hard_policy = HardPolicy::from_host();
    #[cfg(target_os = "macos")]
    let seatbelt_ready = Path::new(SANDBOX_EXEC).is_file()
        && hard_policy
            .as_ref()
            .is_ok_and(|policy| self_test(policy).is_ok());
    #[cfg(target_os = "macos")]
    let denial_collector = seatbelt_ready
        .then(DenialCollector::start)
        .transpose()
        .ok()
        .flatten();
    #[cfg(not(target_os = "macos"))]
    let denial_collector = None;
    #[cfg(target_os = "macos")]
    let can_exec = seatbelt_ready && denial_collector.is_some();
    #[cfg(not(target_os = "macos"))]
    let can_exec = false;
    let runtime = Runtime::new_with_collector(io::stdout(), denial_collector);
    runtime.send(&ServerEvent::Ready {
        version: PROTOCOL_VERSION,
        platform: std::env::consts::OS.to_owned(),
        backend: if can_exec {
            "seatbelt".to_owned()
        } else {
            "unavailable".to_owned()
        },
        can_exec,
        max_frame_bytes: MAX_FRAME_BYTES as u64,
    })?;

    loop {
        let Some(request) = read_frame::<ClientRequest>(&mut reader)? else {
            runtime.shutdown();
            return Ok(());
        };
        match request {
            ClientRequest::Shutdown => {
                runtime.shutdown();
                return Ok(());
            }
            ClientRequest::Exec(request) => {
                let id = Some(request.id.clone());
                if !can_exec {
                    runtime.send(&ServerEvent::Error {
                        id,
                        code: ErrorCode::BackendUnavailable,
                        message: "the Seatbelt backend is unavailable; command blocked".to_owned(),
                    })?;
                    continue;
                }
                let hard_policy = hard_policy
                    .as_ref()
                    .expect("can_exec requires a valid hard policy");
                match validate_exec(request, hard_policy) {
                    Ok(request) => {
                        let id = request.id.clone();
                        if let Err((code, message)) = runtime.start(request) {
                            runtime.send(&ServerEvent::Error {
                                id: Some(id),
                                code,
                                message,
                            })?;
                        }
                    }
                    Err(message) => runtime.send(&ServerEvent::Error {
                        id,
                        code: ErrorCode::InvalidRequest,
                        message,
                    })?,
                }
            }
            ClientRequest::Cancel { id } => {
                if let Err((code, message)) = runtime.cancel(&id) {
                    runtime.send(&ServerEvent::Error {
                        id: Some(id),
                        code,
                        message,
                    })?;
                }
            }
        }
    }
}
