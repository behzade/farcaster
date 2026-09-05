use std::{
    io::{BufRead as _, BufReader},
    process::Child,
    thread,
};

use serde_json::Value;
use tpt_logfmt_parse::LogfmtParser;
use zlog::log_impl::Level;

pub(super) fn capture(child: &mut Child, label: &'static str) -> Result<(), String> {
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("{label} stderr must be piped"))?;
    let reader = thread::Builder::new()
        .name(format!("farcaster-{label}-stderr"))
        .spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut buffer = Vec::new();
            loop {
                buffer.clear();
                match reader.read_until(b'\n', &mut buffer) {
                    Ok(0) => return,
                    Ok(_) => emit_line(label, &String::from_utf8_lossy(&buffer)),
                    Err(error) => {
                        zlog::error!("failed to read {label} stderr: {error}");
                        return;
                    }
                }
            }
        });
    if let Err(error) = reader {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("start {label} stderr reader: {error}"));
    }
    Ok(())
}

fn emit_line(label: &str, line: &str) {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return;
    }
    // Unit-test binaries do not initialize the application's file logger. Keep
    // live harness startup diagnostics visible under --nocapture as well.
    #[cfg(test)]
    eprintln!("{label} stderr: {line}");
    let level = structured_level(line).unwrap_or(Level::Warn);
    zlog::log!(zlog::default_logger!(), level, "{label} stderr: {line}");
}

fn structured_level(line: &str) -> Option<Level> {
    if let Ok(Value::Object(fields)) = serde_json::from_str(line)
        && let Some(level) = fields
            .iter()
            .find_map(|(key, value)| level_from_field(key, value.as_str()?))
    {
        return Some(level);
    }
    LogfmtParser::new(line).find_map(|pair| {
        let (key, value) = pair.ok()?;
        level_from_field(key.as_ref(), value.as_ref())
    })
}

fn level_from_field(key: &str, value: &str) -> Option<Level> {
    matches!(key, "level" | "lvl" | "severity")
        .then(|| value.trim().parse().ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::{Level, structured_level};

    #[test]
    fn structured_stderr_uses_embedded_level() {
        assert_eq!(
            structured_level(
                r#"timestamp=2026-09-04T08:22:46.865Z level=INFO message="spawning process""#
            ),
            Some(Level::Info)
        );
        assert_eq!(
            structured_level(r#"{"level":"error","msg":"boom"}"#),
            Some(Level::Error)
        );
        assert_eq!(structured_level("not a structured log line"), None);
    }
}
