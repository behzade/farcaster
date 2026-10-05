use std::{
    io::Write,
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

pub(super) struct Speech(mpsc::Sender<()>);

impl Speech {
    pub(super) fn start(
        text: String,
    ) -> Result<(Self, async_channel::Receiver<Result<(), String>>), String> {
        let (cancel, stop) = mpsc::channel();
        let (finished, done) = async_channel::bounded(1);
        std::thread::Builder::new()
            .name("voice-playback".into())
            .spawn(move || {
                let result = play(text, stop);
                let _ = finished.send_blocking(result);
            })
            .map_err(|error| format!("Start speech: {error}"))?;
        Ok((Self(cancel), done))
    }
}

impl Drop for Speech {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

fn play(text: String, stop: mpsc::Receiver<()>) -> Result<(), String> {
    if stop.try_recv().is_ok() {
        return Ok(());
    }
    let mut child = Command::new("/usr/bin/say")
        .args(["-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Start macOS speech: {error}"))?;
    let result = (|| {
        child
            .stdin
            .take()
            .ok_or("Speech input unavailable")?
            .write_all(text.as_bytes())
            .map_err(|e| e.to_string())?;
        loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                return if status.success() {
                    Ok(())
                } else {
                    Err("Speech playback failed".into())
                };
            }
            if !matches!(
                stop.recv_timeout(Duration::from_millis(20)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                return Ok(());
            }
        }
    })();
    let _ = child.kill();
    let _ = child.wait();
    result
}

pub(super) fn spoken_opening(answer: &str) -> Option<String> {
    let mut code = false;
    let mut lines = Vec::new();
    for line in answer.lines() {
        let line = line.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            code = !code;
            continue;
        }
        if code || line.starts_with('#') {
            continue;
        }
        if line.is_empty() {
            if !lines.is_empty() {
                break;
            }
            continue;
        }
        lines.push(line);
    }
    let text = lines.join(" ").replace(['*', '`'], "");
    let words = text.split_whitespace().collect::<Vec<_>>();
    if words.is_empty() {
        return None;
    }
    if words.len() <= 80 {
        return Some(text);
    }
    let end = words[..80]
        .iter()
        .rposition(|word| word.ends_with(['.', '!', '?']))
        .map_or(80, |index| index + 1);
    Some(format!(
        "{} There is more in the chat.",
        words[..end].join(" ")
    ))
}

#[cfg(test)]
#[path = "voice_speech_tests.rs"]
mod tests;
