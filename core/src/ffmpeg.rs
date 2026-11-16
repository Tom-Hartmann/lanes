//! Runs the bundled streaming media worker, without a shell or visible console.
use std::{io::{BufRead, BufReader, Read}, path::PathBuf, process::{Command, Stdio}, sync::atomic::{AtomicBool, Ordering}, time::Duration};
use std::os::windows::process::CommandExt;
use crate::{Error, Result};

pub struct Control<'a> {
    pub cancel: &'a AtomicBool,
    pub progress: &'a (dyn Fn(f64) + Sync),
}

pub fn executable() -> Result<PathBuf> {
    let path = std::env::current_exe()?.parent().ok_or_else(|| Error::Invalid("missing application directory".into()))?.join("ffmpeg.exe");
    if !path.is_file() { return Err(Error::Unsupported("The bundled media worker is missing. Reinstall Lanes.".into())); }
    Ok(path)
}

pub fn command() -> Result<Command> {
    let mut cmd = Command::new(executable()?);
    cmd.creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .args(["-hide_banner", "-nostdin", "-loglevel", "error", "-y", "-threads", "2", "-filter_complex_threads", "2"]);
    Ok(cmd)
}

pub fn run(mut cmd: Command, control: Option<&Control<'_>>, duration: f64) -> Result<()> {
    if control.is_some_and(|c| c.cancel.load(Ordering::Relaxed)) { return Err(Error::Invalid("Export cancelled".into())); }
    cmd.args(["-progress", "pipe:1", "-nostats"]).stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null());
    let mut child = cmd.spawn()?;
    let stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    // Keep just the latest timestamp and a bounded diagnostic tail.
    let latest = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let clock = latest.clone();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(std::result::Result::ok) {
            if let Some(value) = line.strip_prefix("out_time_us=").and_then(|v| v.parse::<u64>().ok()) {
                clock.store(value, Ordering::Relaxed);
            }
        }
    });
    let errors = std::thread::spawn(move || {
        let mut tail = Vec::new();
        let mut buffer = [0u8; 4096];
        while let Ok(n) = stderr.read(&mut buffer) {
            if n == 0 { break; }
            tail.extend_from_slice(&buffer[..n]);
            if tail.len() > 16384 { tail.drain(..tail.len() - 16384); }
        }
        String::from_utf8_lossy(&tail).into_owned()
    });
    let status = loop {
        if control.is_some_and(|c| c.cancel.load(Ordering::Relaxed)) {
            let _ = child.kill();
            let _ = child.wait();
            break Err(Error::Invalid("Export cancelled".into()));
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {}
            Err(e) => { let _ = child.kill(); let _ = child.wait(); break Err(e.into()); }
        }
        if let Some(c) = control { (c.progress)((latest.load(Ordering::Relaxed) as f64 / 1e6 / duration.max(0.001)).clamp(0.0, 0.99)); }
        std::thread::sleep(Duration::from_millis(100));
    };
    let _ = reader.join();
    let diagnostic = errors.join().unwrap_or_default();
    if !status?.success() { return Err(Error::Invalid(format!("Media export failed: {}", diagnostic.trim()))); }
    if let Some(c) = control { (c.progress)(1.0); }
    Ok(())
}
