use base64::Engine as _;
use std::io;
#[cfg(target_os = "macos")]
use std::path::Path;

#[cfg(target_os = "macos")]
use pi_sandbox_broker::denial_collector::DenialCollector;
use pi_sandbox_broker::executor::Runtime;
use pi_sandbox_broker::framing::read_frame;
#[cfg(target_os = "linux")]
use pi_sandbox_broker::linux;
use pi_sandbox_broker::protocol::{
    ClientRequest, ErrorCode, MAX_FRAME_BYTES, PROTOCOL_VERSION, ServerEvent,
};
use pi_sandbox_broker::seatbelt::HardPolicy;
#[cfg(target_os = "macos")]
use pi_sandbox_broker::seatbelt::{SANDBOX_EXEC, self_test};
use pi_sandbox_broker::validation::validate_exec;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "linux")]
    {
        let arguments = std::env::args().skip(1).collect::<Vec<_>>();
        if arguments
            .first()
            .is_some_and(|item| item == "__linux_proxy_launch")
        {
            let code = linux::run_proxy_launcher(&arguments[1..])?;
            std::process::exit(code);
        }
    }
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
    #[cfg(target_os = "linux")]
    let can_exec = hard_policy.is_ok() && linux::self_test().is_ok();
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let can_exec = false;
    #[cfg(target_os = "macos")]
    let backend_name = "seatbelt";
    #[cfg(target_os = "linux")]
    let backend_name = "bubblewrap";
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let backend_name = "native";
    let runtime = Runtime::new_with_collector(io::stdout(), denial_collector);
    runtime.send(&ServerEvent::Ready {
        version: PROTOCOL_VERSION,
        platform: std::env::consts::OS.to_owned(),
        backend: if can_exec {
            backend_name.to_owned()
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
        if handle_request(request, &runtime, can_exec, backend_name, &hard_policy)? {
            return Ok(());
        }
    }
}

fn handle_request(
    request: ClientRequest,
    runtime: &Runtime,
    can_exec: bool,
    backend_name: &str,
    hard_policy: &Result<HardPolicy, String>,
) -> Result<bool, String> {
    match request {
        ClientRequest::Shutdown => {
            runtime.shutdown();
            Ok(true)
        }
        ClientRequest::Exec(request) => {
            let id = request.id.clone();
            if !can_exec {
                return send_error(
                    runtime,
                    id,
                    ErrorCode::BackendUnavailable,
                    format!("the {backend_name} backend is unavailable; command blocked"),
                );
            }
            let hard_policy = hard_policy
                .as_ref()
                .expect("can_exec requires a valid hard policy");
            match validate_exec(request, hard_policy) {
                Ok(request) => {
                    let id = request.id.clone();
                    if let Err((code, message)) = runtime.start(request) {
                        return send_error(runtime, id, code, message);
                    }
                    Ok(false)
                }
                Err(message) => send_error(runtime, id, ErrorCode::InvalidRequest, message),
            }
        }
        ClientRequest::Cancel { id } => match runtime.cancel(&id) {
            Ok(()) => Ok(false),
            Err((code, message)) => send_error(runtime, id, code, message),
        },
        ClientRequest::WriteStdin { id, data_base64 } => {
            let result = base64::engine::general_purpose::STANDARD
                .decode(data_base64)
                .map_err(|error| {
                    (
                        ErrorCode::InvalidRequest,
                        format!("invalid stdin base64: {error}"),
                    )
                })
                .and_then(|data| runtime.write_stdin(&id, &data));
            match result {
                Ok(()) => Ok(false),
                Err((code, message)) => send_error(runtime, id, code, message),
            }
        }
    }
}

fn send_error(
    runtime: &Runtime,
    id: String,
    code: ErrorCode,
    message: String,
) -> Result<bool, String> {
    runtime.send(&ServerEvent::Error {
        id: Some(id),
        code,
        message,
    })?;
    Ok(false)
}
