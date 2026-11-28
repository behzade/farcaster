use std::{
    io::{self, BufReader, Read as _, Write as _},
    net::Shutdown,
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

use rmpv::{Value, decode::read_value_with_max_depth, encode::write_value};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_RETRY_INTERVAL: Duration = Duration::from_millis(25);
const MAX_MESSAGE_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq, serde::Deserialize)]
pub(in crate::app::workspace) struct ReviewSelection {
    pub(in crate::app::workspace) list_id: u64,
    pub(in crate::app::workspace) selected: usize,
}

pub(super) struct NvimRpc {
    socket: PathBuf,
    selections: async_channel::Sender<ReviewSelection>,
    connection: Option<Connection>,
}

impl NvimRpc {
    pub(super) fn new(socket: PathBuf, selections: async_channel::Sender<ReviewSelection>) -> Self {
        Self {
            socket,
            selections,
            connection: None,
        }
    }

    pub(super) fn exec(&mut self, code: &str, args: Vec<Value>) -> Result<Value, String> {
        self.exec_with_timeout(code, args, REQUEST_TIMEOUT)
    }

    fn exec_with_timeout(
        &mut self,
        code: &str,
        args: Vec<Value>,
        timeout: Duration,
    ) -> Result<Value, String> {
        let deadline = Instant::now() + timeout;
        let mut connection = match self.connection.take() {
            Some(connection) => connection,
            None => self.connect(deadline)?,
        };
        // After a transport failure we cannot tell if Neovim ran the command.
        // Drop the connection and reconnect on the next request.
        let reply = connection.request(
            "nvim_exec_lua",
            vec![code.into(), Value::Array(args)],
            deadline,
        )?;
        self.connection = Some(connection);
        reply.into_result()
    }

    fn connect(&self, deadline: Instant) -> Result<Connection, String> {
        let socket = loop {
            let remaining =
                remaining_time(deadline).map_err(|error| rpc_io_error("connect to", error))?;
            match UnixStream::connect(&self.socket) {
                Ok(socket) => break socket,
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                    ) =>
                {
                    std::thread::sleep(remaining.min(CONNECT_RETRY_INTERVAL));
                }
                Err(error) => return Err(rpc_io_error("connect to", error)),
            }
        };

        let reader = socket
            .try_clone()
            .map_err(|error| rpc_io_error("connect to", error))?;
        let (send, responses) = mpsc::sync_channel(1);
        let selections = self.selections.clone();
        std::thread::Builder::new()
            .name("neovim-rpc-reader".to_owned())
            .spawn(move || {
                let mut reader = BufReader::new(reader);
                if let Err(error) = read_messages(&mut reader, &send, &selections) {
                    let _ = send.try_send(Err(error));
                }
                let _ = reader.get_ref().shutdown(Shutdown::Both);
            })
            .map_err(|error| format!("Start Neovim RPC reader: {error}"))?;

        let mut connection = Connection {
            socket,
            responses,
            next_id: 0,
        };
        let info = connection
            .request("nvim_get_api_info", vec![], deadline)?
            .into_result()?;
        let channel = info
            .as_array()
            .and_then(|info| info.first())
            .and_then(Value::as_u64)
            .ok_or_else(|| "Neovim did not return an RPC channel ID".to_owned())?;
        connection
            .request(
                "nvim_set_var",
                vec!["farcaster_rpc_channel".into(), channel.into()],
                deadline,
            )?
            .into_result()?;
        Ok(connection)
    }
}

struct Reply {
    id: u64,
    error: Value,
    result: Value,
}

impl Reply {
    fn into_result(self) -> Result<Value, String> {
        if self.error.is_nil() {
            return Ok(self.result);
        }
        let message = self
            .error
            .as_array()
            .and_then(|parts| parts.get(1))
            .and_then(Value::as_str);
        Err(match message {
            Some(message) => format!("Neovim request failed: {message}"),
            None => format!("Neovim request failed: {}", self.error),
        })
    }
}

struct Connection {
    socket: UnixStream,
    responses: mpsc::Receiver<Result<Reply, String>>,
    next_id: u32,
}

impl Connection {
    fn request(
        &mut self,
        method: &str,
        params: Vec<Value>,
        deadline: Instant,
    ) -> Result<Reply, String> {
        let id = self.next_id;
        self.next_id = id.wrapping_add(1);
        let mut request = Vec::new();
        write_value(
            &mut request,
            &Value::Array(vec![
                0.into(),
                id.into(),
                method.into(),
                Value::Array(params),
            ]),
        )
        .map_err(|error| format!("Encode Neovim request: {error}"))?;
        DeadlineWriter {
            socket: &self.socket,
            deadline,
        }
        .write_all(&request)
        .map_err(|error| rpc_io_error("send a request to", error))?;

        let remaining = remaining_time(deadline)
            .map_err(|error| rpc_io_error("read a response from", error))?;
        let reply = self
            .responses
            .recv_timeout(remaining)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => "Neovim remote request timed out".to_owned(),
                mpsc::RecvTimeoutError::Disconnected => {
                    "Neovim closed the RPC connection".to_owned()
                }
            })??;
        if reply.id != u64::from(id) {
            return Err("Neovim returned an unexpected RPC response ID".to_owned());
        }
        Ok(reply)
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        // Close the reader's copy too, including when it is waiting for an event.
        let _ = self.socket.shutdown(Shutdown::Both);
    }
}

fn read_messages(
    reader: &mut BufReader<UnixStream>,
    responses: &mpsc::SyncSender<Result<Reply, String>>,
    selections: &async_channel::Sender<ReviewSelection>,
) -> Result<(), String> {
    loop {
        let message = read_value_with_max_depth(&mut reader.by_ref().take(MAX_MESSAGE_BYTES), 32)
            .map_err(|error| format!("Read Neovim RPC message: {error}"))?;
        let Value::Array(fields) = message else {
            return Err("Invalid Neovim RPC message".to_owned());
        };
        match <[Value; 4]>::try_from(fields) {
            Ok([kind, id, error, result]) if kind.as_u64() == Some(1) => {
                let id = id.as_u64().ok_or("Invalid Neovim RPC response ID")?;
                responses
                    .try_send(Ok(Reply { id, error, result }))
                    .map_err(|_| "Unexpected extra Neovim RPC response".to_owned())?;
            }
            Err(fields) => {
                let Ok([kind, method, params]) = <[Value; 3]>::try_from(fields) else {
                    return Err("Unexpected Neovim RPC message".to_owned());
                };
                if kind.as_u64() != Some(2) {
                    return Err("Unexpected Neovim RPC message".to_owned());
                }
                if method.as_str() == Some("farcaster_review_selection")
                    && let Value::Array(args) = params
                    && let Ok([payload]) = <[Value; 1]>::try_from(args)
                    && let Ok(selection) = rmpv::ext::from_value(payload)
                {
                    let _ = selections.try_send(selection);
                }
            }
            _ => return Err("Unexpected Neovim RPC message".to_owned()),
        }
    }
}

struct DeadlineWriter<'a> {
    socket: &'a UnixStream,
    deadline: Instant,
}

impl io::Write for DeadlineWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.socket
            .set_write_timeout(Some(remaining_time(self.deadline)?))?;
        self.socket.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.socket.flush()
    }
}

fn remaining_time(deadline: Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))
}

fn rpc_io_error(action: &str, error: io::Error) -> String {
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => {
            "Neovim remote request timed out".to_owned()
        }
        _ => format!("Could not {action} Neovim: {error}"),
    }
}

#[cfg(test)]
#[path = "neovim_rpc_tests.rs"]
mod tests;
