use std::{io::Read as _, process::Child, thread};

pub(super) fn capture(child: &mut Child, label: &'static str) -> Result<(), String> {
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("{label} stderr must be piped"))?;
    let reader = thread::Builder::new()
        .name(format!("farcaster-{label}-stderr"))
        .spawn(move || {
            let mut buffer = [0_u8; 8 * 1024];
            loop {
                match stderr.read(&mut buffer) {
                    Ok(0) => return,
                    Ok(read) => {
                        let chunk = String::from_utf8_lossy(&buffer[..read]);
                        let chunk = chunk.trim_end_matches(['\r', '\n']);
                        if !chunk.is_empty() {
                            zlog::warn!("{label} stderr: {chunk}");
                        }
                    }
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
