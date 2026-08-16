use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 4;
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[allow(clippy::large_enum_variant)]
pub enum ClientRequest {
    Exec(ExecRequest),
    Cancel { id: String },
    WriteStdin { id: String, data_base64: String },
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecRequest {
    pub id: String,
    pub command: CommandSpec,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub interactive: bool,
    pub policy: SandboxPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxPolicy {
    pub base_rights: Vec<FilesystemRight>,
    /// Validated rights from the immutable active project-policy snapshot for this command.
    pub grants: Vec<FilesystemRight>,
    pub denies: Vec<FilesystemDeny>,
    pub network: NetworkPolicy,
    /// Trusted machine-configured Unix socket paths. These never come from a
    /// command declaration or project config.
    pub unix_socket_roots: Vec<String>,
    pub output_limit_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FilesystemRight {
    pub access: Access,
    pub path: String,
    pub scope: PathScope,
    pub missing_path: MissingPathBehavior,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FilesystemDeny {
    pub access: DeniedAccess,
    pub pattern: String,
    pub scope: DenyScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeniedAccess {
    Read,
    Write,
    ReadWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PathScope {
    File,
    Tree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyScope {
    File,
    Tree,
    Glob,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingPathBehavior {
    Reject,
    CreateFile,
    CreateTree,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum NetworkPolicy {
    Blocked,
    Loopback,
    Proxy {
        tcp_port: u16,
        unix_socket: String,
        allow_local_binding: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ServerEvent {
    Ready {
        version: u32,
        platform: String,
        backend: String,
        can_exec: bool,
        max_frame_bytes: u64,
    },
    Started {
        id: String,
        pid: u32,
    },
    Stdout {
        id: String,
        sequence: u64,
        data_base64: String,
    },
    Stderr {
        id: String,
        sequence: u64,
        data_base64: String,
    },
    Denials {
        id: String,
        items: Vec<Denial>,
        /// False on macOS because unified log collection is best effort.
        complete: bool,
    },
    Exit {
        id: String,
        code: Option<i32>,
        signal: Option<i32>,
        timed_out: bool,
        cancelled: bool,
        output_truncated: bool,
    },
    Error {
        id: Option<String>,
        code: ErrorCode,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Denial {
    pub operation: String,
    pub path: Option<String>,
    pub process: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BackendUnavailable,
    DuplicateCommandId,
    InvalidRequest,
    PolicyRejected,
    CommandStartFailed,
    Cancelled,
    ProtocolError,
    NotFound,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree_right(access: Access, path: &str) -> FilesystemRight {
        FilesystemRight {
            access,
            path: path.to_owned(),
            scope: PathScope::Tree,
            missing_path: MissingPathBehavior::Reject,
        }
    }

    #[test]
    fn rejects_unknown_request_fields() {
        let input = br#"{"type":"cancel","id":"one","extra":true}"#;
        assert!(serde_json::from_slice::<ClientRequest>(input).is_err());
    }

    #[test]
    fn round_trips_exec_request_without_losing_file_scope() {
        let request = ClientRequest::Exec(ExecRequest {
            id: "command-1".to_owned(),
            command: CommandSpec {
                program: "/bin/bash".to_owned(),
                args: vec!["-c".to_owned(), "printf hello".to_owned()],
            },
            cwd: "/work".to_owned(),
            env: BTreeMap::from([("PATH".to_owned(), "/usr/bin:/bin".to_owned())]),
            timeout_ms: Some(5_000),
            interactive: false,
            policy: SandboxPolicy {
                base_rights: vec![
                    tree_right(Access::Read, "/"),
                    tree_right(Access::Write, "/work"),
                ],
                grants: vec![FilesystemRight {
                    access: Access::Write,
                    path: "/state/tool/database.sqlite".to_owned(),
                    scope: PathScope::File,
                    missing_path: MissingPathBehavior::CreateFile,
                }],
                denies: vec![FilesystemDeny {
                    access: DeniedAccess::ReadWrite,
                    pattern: "/home/user/.ssh".to_owned(),
                    scope: DenyScope::Tree,
                }],
                network: NetworkPolicy::Blocked,
                unix_socket_roots: vec!["/nix/var/nix/daemon-socket/socket".to_owned()],
                output_limit_bytes: 1024,
            },
        });
        let encoded = serde_json::to_vec(&request).expect("serialize request");
        let decoded = serde_json::from_slice(&encoded).expect("deserialize request");
        assert_eq!(request, decoded);
    }

    #[test]
    fn protocol_v4_has_narrow_proxy_and_loopback_grants() {
        let with_proxy = br#"{"mode":"proxy","tcp_port":1234,"unix_socket":"/tmp/proxy.sock","allow_local_binding":true}"#;
        assert_eq!(
            serde_json::from_slice::<NetworkPolicy>(with_proxy).expect("proxy policy"),
            NetworkPolicy::Proxy {
                tcp_port: 1234,
                unix_socket: "/tmp/proxy.sock".to_owned(),
                allow_local_binding: true,
            }
        );
        assert_eq!(
            serde_json::from_slice::<NetworkPolicy>(br#"{"mode":"loopback"}"#)
                .expect("loopback policy"),
            NetworkPolicy::Loopback,
        );
    }
}
