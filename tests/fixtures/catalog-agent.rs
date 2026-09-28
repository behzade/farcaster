use std::{
    io::{self, BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    time::Duration,
};

fn main() -> io::Result<()> {
    let executable = std::env::current_exe()?;
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    std::fs::write(executable.with_extension("args"), args.join("\n"))?;
    if executable.file_name() == Some(std::ffi::OsStr::new("opencode"))
        && args == ["debug", "paths"]
    {
        return Ok(());
    }
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
        std::process::exit(0);
    });
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
