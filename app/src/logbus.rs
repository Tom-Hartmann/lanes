//! Backend log sink. Keeps the most recent entries in a ring buffer and pushes
//! each new one to the UI as a `log` event. The UI can be closed and reopened
//! at any time, so on load it fetches the history with `get_logs`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use log::{Level, LevelFilter, Log, Metadata, Record};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

const CAPACITY: usize = 500;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    /// Increasing sequence number, used by the UI to merge history with live events.
    pub seq: u64,
    /// Wall-clock time in Unix milliseconds.
    pub time_ms: u64,
    pub level: &'static str,
    pub target: String,
    pub message: String,
}

struct LogBus {
    app: OnceLock<AppHandle>,
    ring: Mutex<VecDeque<LogEntry>>,
    seq: AtomicU64,
    /// `%LOCALAPPDATA%\Lanes\lanes.log`, for bug reports.
    file: Mutex<Option<std::fs::File>>,
}

static BUS: LogBus = LogBus {
    app: OnceLock::new(),
    ring: Mutex::new(VecDeque::new()),
    seq: AtomicU64::new(0),
    file: Mutex::new(None),
};

const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

/// Installs the logger. Call once, as early as possible.
pub fn install() {
    let level = if cfg!(debug_assertions) { LevelFilter::Debug } else { LevelFilter::Info };
    *BUS.file.lock().unwrap_or_else(|e| e.into_inner()) = open_log_file();
    if log::set_logger(&BUS).is_ok() {
        log::set_max_level(level);
    }
}

/// Opens the log file for appending; starts it over once it passes 5 MB.
fn open_log_file() -> Option<std::fs::File> {
    let dir = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from)?.join("Lanes");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("lanes.log");
    let too_big = std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_FILE_BYTES);
    std::fs::OpenOptions::new().create(true).append(!too_big).write(true).truncate(too_big).open(path).ok()
}

/// Starts forwarding entries to the UI.
pub fn attach(app: AppHandle) {
    let _ = BUS.app.set(app);
}

/// Returns the buffered history, oldest first.
pub fn history() -> Vec<LogEntry> {
    BUS.ring.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect()
}

impl Log for LogBus {
    fn enabled(&self, metadata: &Metadata) -> bool {
        // Keep third-party crates (tao, wry, ...) quiet below warnings.
        metadata.level() <= Level::Warn || metadata.target().starts_with("lanes")
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let entry = LogEntry {
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            time_ms: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
            level: record.level().as_str(),
            target: record.target().to_owned(),
            message: record.args().to_string(),
        };
        if cfg!(debug_assertions) {
            eprintln!("[{:<5} {}] {}", entry.level, entry.target, entry.message);
        }
        if let Some(f) = self.file.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            use std::io::Write;
            let _ = writeln!(f, "{} [{:<5} {}] {}", entry.time_ms, entry.level, entry.target, entry.message);
        }
        {
            let mut ring = self.ring.lock().unwrap_or_else(|e| e.into_inner());
            if ring.len() == CAPACITY {
                ring.pop_front();
            }
            ring.push_back(entry.clone());
        }
        if let Some(app) = self.app.get() {
            let _ = app.emit("log", entry);
        }
    }

    fn flush(&self) {}
}
