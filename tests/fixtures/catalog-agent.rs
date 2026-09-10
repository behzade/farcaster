//! A std-only subprocess relay. Tests supply protocol replies over a local socket;
//! the real adapter still owns process launch, pipes, framing, and translation.
use std::{
    io::{self, BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    time::Duration,
};

fn main() -> io::Result<()> {
    let executable = std::env::current_exe()?;
    std::fs::write(
        executable.with_extension("args"),
        std::env::args().skip(1).collect::<Vec<_>>().join("\n"),
    )?;
    let mut control = UnixStream::connect(executable.parent().unwrap().join("control.sock"))?;
    control.set_read_timeout(Some(Duration::from_secs(20)))?;
    writeln!(
        control,
        "{}",
        executable.file_name().unwrap().to_string_lossy()
    )?;
    if std::env::var_os("FARCASTER_FIXTURE_REPORT_MODE").is_some() {
        writeln!(
            control,
            "{}",
            if std::env::args().any(|arg| arg == "--print") {
                "print"
            } else {
                "rpc"
            }
        )?;
    }
    let replies = control.try_clone()?;
    let forwarder = std::thread::spawn(move || {
        let mut output = io::stdout().lock();
        for line in BufReader::new(replies).lines() {
            let Ok(line) = line else { break };
            if writeln!(output, "{line}")
                .and_then(|_| output.flush())
                .is_err()
            {
                break;
            }
        }
        // Also exit if the test crashes or its watchdog kills it. Do not leave
        // a child blocked on stdin after the controller closes its socket.
        std::process::exit(0);
    });
    // Pi's print mode has no stdin; keep the relay alive for its output.
    if std::env::var_os("FARCASTER_FIXTURE_REPORT_MODE").is_some()
        && std::env::args().any(|arg| arg == "--print")
    {
        let _ = forwarder.join();
        return Ok(());
    }
    for line in io::stdin().lock().lines() {
        writeln!(control, "{}", line?)?;
    }
    Ok(())
}
