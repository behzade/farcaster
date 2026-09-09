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
    #[cfg(test)]
    eprintln!("{label} stderr: {line}");
    let level = structured_level(line).unwrap_or(Level::Warn);
    zlog::log!(zlog::default_logger!(), level, "{label} stderr: {line}");
}

fn structured_level(line: &str) -> Option<Level> {
    if let Some(level) = glog_level(line) {
        return Some(level);
    }
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

fn glog_level(line: &str) -> Option<Level> {
    let (header, message) = line.split_once("] ")?;
    let mut fields = header.split_whitespace();
    let stamp = fields.next()?.as_bytes();
    if stamp.len() != 5 || !stamp[1..].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let level = match stamp[0] {
        b'I' => Level::Info,
        b'W' => Level::Warn,
        b'E' | b'F' => Level::Error,
        _ => return None,
    };
    let time = fields.next()?.as_bytes();
    if time.len() != 15
        || !time
            .iter()
            .zip(b"00:00:00.000000")
            .all(|(byte, pattern)| match pattern {
                b'0' => byte.is_ascii_digit(),
                _ => byte == pattern,
            })
    {
        return None;
    }
    if !fields.next()?.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let (_, line_number) = fields.next()?.rsplit_once(':')?;
    if line_number.is_empty()
        || !line_number.bytes().all(|byte| byte.is_ascii_digit())
        || fields.next().is_some()
    {
        return None;
    }
    // Antigravity's raw payloads repeat accumulated text on every update.
    if level == Level::Info && message.starts_with("RAW WS MSG: ") {
        return Some(Level::Debug);
    }
    Some(level)
}

fn level_from_field(key: &str, value: &str) -> Option<Level> {
    matches!(key, "level" | "lvl" | "severity")
        .then(|| value.trim().parse().ok())
        .flatten()
}

#[cfg(test)]
#[path = "child_stderr_tests.rs"]
mod tests;
