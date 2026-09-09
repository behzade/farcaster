//! A std-only subprocess relay. Tests supply protocol replies over a local socket;
//! the real adapter still owns process launch, pipes, framing, and translation.
use std::{
    io::{self, BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    time::Duration,
};

fn main() -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let mut control = UnixStream::connect(executable.parent().unwrap().join("control.sock"))?;
    control.set_read_timeout(Some(Duration::from_secs(20)))?;
    writeln!(
        control,
        "{}",
        executable.file_name().unwrap().to_string_lossy()
    )?;
    let replies = control.try_clone()?;
    std::thread::spawn(move || {
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
    for line in io::stdin().lock().lines() {
        writeln!(control, "{}", line?)?;
    }
    Ok(())
}
