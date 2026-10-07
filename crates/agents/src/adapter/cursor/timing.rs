use serde_json::Value;
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Instant,
};

pub(super) fn script(source: &str) -> String {
    format!("{}\n{source}", include_str!("timing.mjs"))
}

// Accept only our timing schema; never forward arbitrary SDK stderr or error text.
pub(super) fn log_event(event: &Value) {
    let (Some(pid), Some(id), Some(operation), Some(phase), Some(elapsed)) = (
        event["pid"].as_u64(),
        event["id"].as_u64(),
        event["operation"].as_str(),
        event["phase"].as_str(),
        event["elapsedMs"].as_f64(),
    ) else {
        return;
    };
    if !matches!(
        operation,
        "SDK.import"
            | "SqliteLocalAgentStore.import"
            | "SqliteLocalAgentStore.open"
            | "SqliteLocalAgentStore.dispose"
            | "Cursor.configure"
            | "Cursor.models.list"
            | "Agent.create"
            | "Agent.resume"
            | "Agent.get"
            | "Agent.listRuns"
            | "Agent.messages.list"
            | "Agent.send"
            | "Agent.close"
            | "Run.lifecycle"
            | "Run.stream"
            | "Run.stream.consume"
            | "Run.wait"
            | "Run.steer"
            | "Run.cancel"
            | "InMemoryCredentialStore.new"
            | "InMemoryCredentialStore.load"
            | "FileCredentialStore.new"
            | "FileCredentialStore.save"
            | "Cursor.auth.login"
    ) || !matches!(
        phase,
        "start" | "ok" | "error" | "first_delta" | "first_text" | "first_message"
    ) || !elapsed.is_finite()
        || elapsed < 0.0
    {
        return;
    }
    zlog::info!(
        "Cursor SDK timing pid={pid} call={id} operation={operation} phase={phase} elapsed_ms={elapsed:.3}"
    );
}

static NEXT_CALL: AtomicU64 = AtomicU64::new(1);

pub(super) struct Call {
    id: u64,
    operation: String,
    start: Instant,
    finished: bool,
}

impl Call {
    pub(super) fn start(operation: impl Into<String>) -> Self {
        let call = Self {
            id: NEXT_CALL.fetch_add(1, Ordering::Relaxed),
            operation: operation.into(),
            start: Instant::now(),
            finished: false,
        };
        call.record("start");
        call
    }

    pub(super) fn record(&self, phase: &str) {
        zlog::info!(
            "Cursor bridge timing call={} operation={} phase={} elapsed_ms={:.3}",
            self.id,
            self.operation,
            phase,
            self.start.elapsed().as_secs_f64() * 1000.0
        );
    }

    pub(super) fn finish(&mut self, success: bool) {
        if !self.finished {
            self.record(if success { "ok" } else { "error" });
            self.finished = true;
        }
    }
}

impl Drop for Call {
    fn drop(&mut self) {
        if !self.finished {
            self.record("dropped");
        }
    }
}

#[cfg(test)]
#[path = "timing_tests.rs"]
mod tests;
