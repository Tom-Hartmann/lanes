//! Admin mode: an elevated Lanes started from a scheduled task.
//!
//! A standard-user process cannot see keyboard input aimed at elevated games
//! (UIPI), so Lanes can register a Task Scheduler task that starts it with the
//! user's elevated token: at logon, or on demand from the non-elevated app.
//! Starting a task needs no UAC prompt, so the prompt appears exactly once,
//! when the user turns the feature on.
//!
//! A task that runs elevated without a prompt is only safe if nobody but an
//! administrator can replace the program it starts. Before registering,
//! [`install_task`] therefore insists that the executable
//! - is the one actually running (not a path handed in from outside),
//! - lives under Program Files, which standard users cannot write to, and
//! - carries a valid Authenticode signature.
//!
//! Flow:
//! 1. The non-elevated app calls [`install_via_uac`]. It relaunches `lanes.exe`
//!    with `--admin-task install ...` through the "runas" verb and waits.
//! 2. That elevated copy calls [`handle_cli`] first thing in `main()`, which
//!    runs [`install_task`] and exits with a code from [`exit_code`].
//! 3. Later, the app calls [`run_task`] (or Windows does, at logon) and the task
//!    starts `lanes.exe --from-task` elevated.

mod scheduler;

use std::io;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::ERROR_CANCELLED;
use windows::Win32::UI::Shell::{FOLDERID_ProgramFiles, FOLDERID_ProgramFilesX86};
use windows::core::HRESULT;

use crate::sys::com::{self, ComApartment};
use crate::sys::{shell, token, trust};
use crate::paths;
use scheduler::{Scheduler, TaskSpec};

/// Task Scheduler folder holding the Lanes task.
pub const TASK_FOLDER: &str = "\\Lanes";
/// Name of the task inside [`TASK_FOLDER`].
pub const TASK_NAME: &str = "Elevated";
/// Command-line switch that runs an admin-task command and exits; see [`handle_cli`].
pub const ADMIN_TASK_ARG: &str = "--admin-task";
/// Argument the task passes to `lanes.exe` so the app knows it was started elevated by the task.
pub const FROM_TASK_ARG: &str = "--from-task";
/// Debug builds only: set to `1` to let [`verify_signature`] accept unsigned files.
pub const ALLOW_UNSIGNED_ENV: &str = "LANES_ALLOW_UNSIGNED_ADMIN";

/// Task Scheduler priority 4 maps to a normal-priority process. The default (7)
/// would start Lanes below normal priority, which hurts capture.
const TASK_PRIORITY: i32 = 4;
/// ISO 8601 duration meaning "no execution time limit" (the default is 72 hours).
const NO_TIME_LIMIT: &str = "PT0S";

/// Exit codes of the `--admin-task` helper. [`install_via_uac`] and
/// [`remove_via_uac`] report any non-zero code as [`AdminError::HelperFailed`].
pub mod exit_code {
    pub const OK: i32 = 0;
    /// Any failure without a more specific code (COM, I/O, ...). See the log.
    pub const FAILED: i32 = 1;
    /// The `--admin-task` arguments could not be parsed.
    pub const USAGE: i32 = 2;
    pub const NOT_ELEVATED: i32 = 3;
    pub const NOT_PROTECTED_LOCATION: i32 = 4;
    pub const UNSIGNED: i32 = 5;
    /// The UAC prompt was answered with a different account's credentials.
    pub const USER_MISMATCH: i32 = 6;
}

/// How the elevated task starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskOptions {
    /// Start Lanes elevated when the user logs on.
    pub logon_trigger: bool,
    /// Delay after logon before starting, in seconds (0 = none).
    pub delay_secs: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum AdminError {
    #[error("{} is not installed under Program Files", .0.display())]
    NotProtectedLocation(PathBuf),
    #[error("signature check failed: {0}")]
    Unsigned(String),
    #[error("{} is not the running executable", .0.display())]
    NotCurrentExe(PathBuf),
    #[error("this operation requires administrator rights")]
    NotElevated,
    #[error("the administrator prompt was declined")]
    Cancelled,
    #[error("the elevated task is not installed")]
    NotInstalled,
    #[error("elevated as a different account ({0}); sign in with your own account in the prompt")]
    UserMismatch(String),
    #[error("Task Scheduler: {0}")]
    Com(#[from] windows::core::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("the elevated helper failed with exit code {0}")]
    HelperFailed(i32),
}

impl AdminError {
    /// The helper's exit code for this error.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::NotElevated => exit_code::NOT_ELEVATED,
            Self::NotProtectedLocation(_) | Self::NotCurrentExe(_) => exit_code::NOT_PROTECTED_LOCATION,
            Self::Unsigned(_) => exit_code::UNSIGNED,
            Self::UserMismatch(_) => exit_code::USER_MISMATCH,
            _ => exit_code::FAILED,
        }
    }
}

// ---------------------------------------------------------------------------
// Non-elevated side
// ---------------------------------------------------------------------------

/// Installs (or updates) the task through one UAC prompt. Called by the
/// non-elevated app; blocks until the elevated helper exits, so call it off the UI thread.
///
/// The location and signature checks run here first as well, so an install
/// that would be refused anyway does not show a prompt.
pub fn install_via_uac(opts: &TaskOptions) -> Result<(), AdminError> {
    let exe = std::env::current_exe()?;
    verify_install_location(&exe)?;
    verify_signature(&exe)?;
    // Tell the helper who we are, so it can refuse to register the task for
    // whatever account the user typed into an over-the-shoulder prompt.
    let expect_user = token::current_user_sid().ok();
    run_helper_elevated(&exe, &Command::Install { opts: opts.clone(), expect_user })
}

/// Removes the task through one UAC prompt. Blocks like [`install_via_uac`].
pub fn remove_via_uac() -> Result<(), AdminError> {
    let exe = std::env::current_exe()?;
    run_helper_elevated(&exe, &Command::Remove)
}

fn run_helper_elevated(exe: &Path, command: &Command) -> Result<(), AdminError> {
    let parameters = command.to_args().join(" ");
    match shell::run_elevated_and_wait(exe, &parameters) {
        Ok(0) => Ok(()),
        // Exit codes are DWORDs; reinterpret NTSTATUS-style values as negative i32 like Windows tools do.
        Ok(code) => Err(AdminError::HelperFailed(code as i32)),
        Err(e) if e.code() == HRESULT::from_win32(ERROR_CANCELLED.0) => Err(AdminError::Cancelled),
        Err(e) => Err(e.into()),
    }
}

/// True if the task is registered and visible to the caller. Works non-elevated
/// because the task grants the user read access.
pub fn task_exists() -> bool {
    // The scheduler owns the COM initialization, so it must outlive every
    // interface obtained from it. (Dropping it inside a closure while the task
    // was still alive crashed Lanes at startup on Tauri's main thread.)
    let scheduler = match Scheduler::connect() {
        Ok(s) => s,
        Err(e) => {
            log::debug!("connecting to Task Scheduler: {e}");
            return false;
        }
    };
    let exists = match scheduler.task(TASK_FOLDER, TASK_NAME) {
        Ok(task) => task.is_some(),
        Err(e) => {
            log::debug!("looking up {TASK_FOLDER}\\{TASK_NAME}: {e}");
            false
        }
    };
    drop(scheduler);
    exists
}

/// Starts the task now (`IRegisteredTask::Run`). No UAC prompt: the task's
/// security descriptor lets the user execute it.
pub fn run_task() -> Result<(), AdminError> {
    let scheduler = Scheduler::connect()?;
    let task = scheduler.task(TASK_FOLDER, TASK_NAME)?.ok_or(AdminError::NotInstalled)?;
    scheduler::run(&task)?;
    Ok(())
}

/// True if the running executable lives in a protected (admin-only) location,
/// i.e. Lanes was installed per-machine. Admin mode is unavailable otherwise.
pub fn installed_per_machine() -> bool {
    std::env::current_exe().is_ok_and(|exe| verify_install_location(&exe).is_ok())
}

// ---------------------------------------------------------------------------
// Elevated side
// ---------------------------------------------------------------------------

/// Runs an `--admin-task` command if `args` contains one and returns the exit
/// code the process should exit with. Returns `None` otherwise.
///
/// `main()` calls this before anything else (in particular before the
/// single-instance lock, which the helper must not take). `args` is the full
/// argument list, `std::env::args().collect()`.
pub fn handle_cli(args: &[String]) -> Option<i32> {
    let command = parse_cli(args)?;
    let code = match command {
        Err(msg) => {
            log::error!("{ADMIN_TASK_ARG}: {msg}");
            exit_code::USAGE
        }
        Ok(command) => match run_helper(&command) {
            Ok(()) => {
                log::info!("{ADMIN_TASK_ARG} {}: done", command.verb());
                exit_code::OK
            }
            Err(e) => {
                log::error!("{ADMIN_TASK_ARG} {}: {e}", command.verb());
                e.exit_code()
            }
        },
    };
    Some(code)
}

fn run_helper(command: &Command) -> Result<(), AdminError> {
    if !token::is_elevated() {
        return Err(AdminError::NotElevated);
    }
    // We are a fresh, short-lived process: set COM security before anything else uses COM.
    let _com = ComApartment::init()?;
    com::init_process_security();
    match command {
        Command::Install { opts, expect_user } => {
            if let Some(expected) = expect_user {
                let actual = token::current_user_sid()?;
                if !actual.eq_ignore_ascii_case(expected) {
                    return Err(AdminError::UserMismatch(actual));
                }
            }
            install_task(&std::env::current_exe()?, opts)
        }
        Command::Remove => remove_task(),
    }
}

/// Registers (or replaces) the elevated task for the current user. Requires elevation.
///
/// The task runs `exe --from-task` with the highest privileges of the account
/// that runs this process, only while that user is logged on (interactive
/// token, so no password is stored). Its security descriptor lets the user
/// read and start it, but only administrators and SYSTEM may change it.
pub fn install_task(exe: &Path, opts: &TaskOptions) -> Result<(), AdminError> {
    if !token::is_elevated() {
        return Err(AdminError::NotElevated);
    }
    let exe = std::fs::canonicalize(exe)?;
    let current = std::fs::canonicalize(std::env::current_exe()?)?;
    if !paths::same_path(&exe, &current) {
        return Err(AdminError::NotCurrentExe(exe));
    }
    verify_install_location(&exe)?;
    let signer = verify_signature(&exe)?;

    let user_sid = token::current_user_sid()?;
    let sddl = task_sddl(&user_sid).ok_or_else(|| io::Error::other(format!("unexpected SID format {user_sid}")))?;
    let exe = paths::strip_verbatim(&exe);
    let working_dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    let delay = opts.logon_trigger.then(|| format_delay(opts.delay_secs));
    let spec = TaskSpec {
        folder: TASK_FOLDER,
        name: TASK_NAME,
        exe: &exe,
        arguments: FROM_TASK_ARG,
        working_dir: &working_dir,
        user_sid: &user_sid,
        sddl: &sddl,
        execution_time_limit: NO_TIME_LIMIT,
        priority: TASK_PRIORITY,
        logon_delay: delay.as_deref(),
        author: "Lanes",
        description: "Starts Lanes with administrator rights so hotkeys work while elevated games have focus.",
    };
    Scheduler::connect()?.register(&spec)?;
    log::info!("registered {TASK_FOLDER}\\{TASK_NAME} for {user_sid} ({}; signed by {signer})", exe.display());
    Ok(())
}

/// Deletes the task, then its folder if empty. Succeeds if nothing is installed.
pub fn remove_task() -> Result<(), AdminError> {
    Scheduler::connect()?.delete(TASK_FOLDER, TASK_NAME)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Checks
// ---------------------------------------------------------------------------

/// Requires `exe` to live under `%ProgramFiles%` or `%ProgramFiles(x86)%`.
///
/// Both sides are canonicalized (links, 8.3 names and `..` resolved) and then
/// compared component by component, ignoring case.
pub fn verify_install_location(exe: &Path) -> Result<(), AdminError> {
    let reject = || AdminError::NotProtectedLocation(exe.to_path_buf());
    let exe = std::fs::canonicalize(exe).map_err(|_| reject())?;
    for id in [&FOLDERID_ProgramFiles, &FOLDERID_ProgramFilesX86] {
        let root = match shell::known_folder(id).and_then(|p| std::fs::canonicalize(p).map_err(Into::into)) {
            Ok(root) => root,
            Err(e) => {
                log::debug!("resolving Program Files: {e}");
                continue;
            }
        };
        if paths::is_within(&exe, &root) {
            return Ok(());
        }
    }
    Err(reject())
}

/// Checks the embedded Authenticode signature of `exe` and returns the signer's name.
///
/// Uses `WinVerifyTrust` with `WINTRUST_ACTION_GENERIC_VERIFY_V2` and no
/// revocation check, so it works offline (see `sys::trust` for why that is
/// acceptable here).
///
/// Debug builds accept an unsigned file when the `LANES_ALLOW_UNSIGNED_ADMIN`
/// environment variable is `1`, to test admin mode without a certificate.
/// Release builds never do.
pub fn verify_signature(exe: &Path) -> Result<String, AdminError> {
    match trust::verify_embedded(exe) {
        Ok(signer) => Ok(signer),
        Err(e) => {
            #[cfg(debug_assertions)]
            if std::env::var(ALLOW_UNSIGNED_ENV).is_ok_and(|v| v == "1") {
                log::warn!("{}: {e}; accepted because {ALLOW_UNSIGNED_ENV}=1 (debug build)", exe.display());
                return Ok("<unsigned, debug override>".to_owned());
            }
            Err(AdminError::Unsigned(format!("{}: {}", exe.display(), e.message())))
        }
    }
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// An `--admin-task` command line.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Install { opts: TaskOptions, expect_user: Option<String> },
    Remove,
}

impl Command {
    fn verb(&self) -> &'static str {
        match self {
            Self::Install { .. } => "install",
            Self::Remove => "remove",
        }
    }

    /// The helper's arguments. Every token is a fixed word, a number or a SID,
    /// none of which contain spaces or quotes, so joining with spaces is a
    /// faithful command line.
    fn to_args(&self) -> Vec<String> {
        let mut args = vec![ADMIN_TASK_ARG.to_owned(), self.verb().to_owned()];
        if let Self::Install { opts, expect_user } = self {
            if opts.logon_trigger {
                args.push("--logon".to_owned());
            }
            if opts.delay_secs > 0 {
                args.extend(["--delay".to_owned(), opts.delay_secs.to_string()]);
            }
            if let Some(sid) = expect_user.as_deref().filter(|s| is_sid(s)) {
                args.extend(["--expect-user".to_owned(), sid.to_owned()]);
            }
        }
        args
    }
}

/// Finds `--admin-task` anywhere in `args` and parses what follows it.
/// `None` if absent; `Some(Err)` describes a malformed command.
fn parse_cli(args: &[String]) -> Option<Result<Command, String>> {
    let start = args.iter().position(|a| a == ADMIN_TASK_ARG)?;
    let mut rest = args[start + 1..].iter().map(String::as_str);
    let result = match rest.next() {
        Some("install") => parse_install(rest),
        Some("remove") => match rest.next() {
            None => Ok(Command::Remove),
            Some(extra) => Err(format!("unexpected argument {extra:?}")),
        },
        Some(other) => Err(format!("unknown command {other:?}")),
        None => Err("missing command (install or remove)".to_owned()),
    };
    Some(result)
}

fn parse_install<'a>(mut rest: impl Iterator<Item = &'a str>) -> Result<Command, String> {
    let mut opts = TaskOptions { logon_trigger: false, delay_secs: 0 };
    let mut expect_user = None;
    while let Some(arg) = rest.next() {
        match arg {
            "--logon" => opts.logon_trigger = true,
            "--delay" => {
                let value = rest.next().ok_or("--delay needs a value")?;
                opts.delay_secs = value.parse().map_err(|_| format!("invalid delay {value:?}"))?;
            }
            "--expect-user" => {
                let sid = rest.next().filter(|s| is_sid(s)).ok_or("--expect-user needs a SID")?;
                expect_user = Some(sid.to_owned());
            }
            other => return Err(format!("unexpected argument {other:?}")),
        }
    }
    Ok(Command::Install { opts, expect_user })
}

/// Formats a delay as the ISO 8601 duration Task Scheduler expects (`PT30S`).
fn format_delay(secs: u32) -> String {
    format!("PT{secs}S")
}

/// A string SID such as `S-1-5-21-1-2-3-1001`: `S`, then dash-separated decimal fields.
fn is_sid(s: &str) -> bool {
    let mut parts = s.split('-');
    parts.next().is_some_and(|p| p.eq_ignore_ascii_case("S"))
        && s.split('-').count() >= 3
        && parts.all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// The task's security descriptor: protected DACL, full control for
/// Administrators and SYSTEM, read + execute (start) for the user.
/// `None` if `user_sid` is not a well-formed SID (it is spliced into SDDL).
fn task_sddl(user_sid: &str) -> Option<String> {
    is_sid(user_sid).then(|| format!("D:P(A;;GA;;;BA)(A;;GA;;;SY)(A;;GRGX;;;{user_sid})"))
}

#[cfg(test)]
mod tests {

    /// Regression: task_exists on a thread without COM (like Tauri's main
    /// thread at startup) released the task after uninitializing COM and
    /// crashed with an access violation when the task existed.
    #[test]
    fn task_exists_is_safe_on_a_thread_without_com() {
        let t = std::thread::spawn(|| (0..3).map(|_| task_exists()).collect::<Vec<_>>());
        let results = t.join().expect("no crash or panic");
        assert!(results.windows(2).all(|w| w[0] == w[1]));
    }

    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    const SID: &str = "S-1-5-21-1004336348-1177238915-682003330-1001";

    #[test]
    fn no_admin_task_flag_means_none() {
        assert_eq!(parse_cli(&args("lanes.exe --minimized")), None);
        assert_eq!(parse_cli(&[]), None);
    }

    #[test]
    fn parses_install_and_remove() {
        assert_eq!(
            parse_cli(&args("lanes.exe --admin-task install")),
            Some(Ok(Command::Install { opts: TaskOptions { logon_trigger: false, delay_secs: 0 }, expect_user: None }))
        );
        assert_eq!(
            parse_cli(&args(&format!("x --admin-task install --delay 30 --logon --expect-user {SID}"))),
            Some(Ok(Command::Install {
                opts: TaskOptions { logon_trigger: true, delay_secs: 30 },
                expect_user: Some(SID.to_owned())
            }))
        );
        assert_eq!(parse_cli(&args("x --admin-task remove")), Some(Ok(Command::Remove)));
    }

    #[test]
    fn rejects_malformed_commands() {
        for bad in [
            "x --admin-task",
            "x --admin-task frobnicate",
            "x --admin-task remove --logon",
            "x --admin-task install --delay",
            "x --admin-task install --delay -5",
            "x --admin-task install --delay soon",
            "x --admin-task install --expect-user bob",
            "x --admin-task install --expect-user",
            "x --admin-task install --wat",
        ] {
            assert!(matches!(parse_cli(&args(bad)), Some(Err(_))), "{bad}");
        }
    }

    #[test]
    fn args_round_trip() {
        for command in [
            Command::Remove,
            Command::Install { opts: TaskOptions { logon_trigger: false, delay_secs: 0 }, expect_user: None },
            Command::Install { opts: TaskOptions { logon_trigger: true, delay_secs: 45 }, expect_user: Some(SID.into()) },
        ] {
            let mut argv = vec!["lanes.exe".to_owned()];
            argv.extend(command.to_args());
            assert_eq!(parse_cli(&argv), Some(Ok(command.clone())));
        }
    }

    #[test]
    fn helper_command_line() {
        let command =
            Command::Install { opts: TaskOptions { logon_trigger: true, delay_secs: 30 }, expect_user: None };
        assert_eq!(command.to_args().join(" "), "--admin-task install --logon --delay 30");
        assert_eq!(Command::Remove.to_args().join(" "), "--admin-task remove");
    }

    #[test]
    fn delay_is_iso8601() {
        assert_eq!(format_delay(0), "PT0S");
        assert_eq!(format_delay(30), "PT30S");
        assert_eq!(format_delay(u32::MAX), "PT4294967295S");
    }

    #[test]
    fn sddl_grants_user_read_execute() {
        assert_eq!(task_sddl(SID).unwrap(), format!("D:P(A;;GA;;;BA)(A;;GA;;;SY)(A;;GRGX;;;{SID})"));
        assert_eq!(task_sddl("S-1-5-18").unwrap(), "D:P(A;;GA;;;BA)(A;;GA;;;SY)(A;;GRGX;;;S-1-5-18)");
    }

    #[test]
    fn sddl_rejects_injection() {
        for bad in ["", "S", "S-1", "S-1-5-21-1)(A;;GA;;;WD", "WD", "S-1--5", "X-1-5-18", "S-1-5-18 "] {
            assert_eq!(task_sddl(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn exit_codes_are_distinct() {
        assert_eq!(AdminError::NotElevated.exit_code(), exit_code::NOT_ELEVATED);
        assert_eq!(AdminError::Unsigned(String::new()).exit_code(), exit_code::UNSIGNED);
        assert_eq!(AdminError::NotProtectedLocation(PathBuf::new()).exit_code(), exit_code::NOT_PROTECTED_LOCATION);
        assert_eq!(AdminError::UserMismatch(String::new()).exit_code(), exit_code::USER_MISMATCH);
        assert_eq!(AdminError::HelperFailed(9).exit_code(), exit_code::FAILED);
    }

    // The checks below call real Win32 APIs but change nothing.

    #[test]
    fn test_binary_is_not_in_program_files() {
        let exe = std::env::current_exe().unwrap();
        assert!(matches!(verify_install_location(&exe), Err(AdminError::NotProtectedLocation(_))));
        assert!(!installed_per_machine());
    }

    #[test]
    fn program_files_file_passes_location_check() {
        let Ok(root) = shell::known_folder(&FOLDERID_ProgramFiles) else { return };
        // Any existing file under Program Files will do.
        let candidate = std::fs::read_dir(&root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|dir| std::fs::read_dir(dir.path()).ok())
            .flatten()
            .flatten()
            .find(|e| e.file_type().is_ok_and(|t| t.is_file()));
        if let Some(entry) = candidate {
            assert!(verify_install_location(&entry.path()).is_ok(), "{}", entry.path().display());
        }
    }

    #[test]
    fn unsigned_test_binary_fails_signature_check() {
        if std::env::var(ALLOW_UNSIGNED_ENV).is_ok_and(|v| v == "1") {
            return;
        }
        let exe = std::env::current_exe().unwrap();
        assert!(matches!(verify_signature(&exe), Err(AdminError::Unsigned(_))));
    }

    #[test]
    fn task_lookup_does_not_fail() {
        // Must not panic or hang whether or not the task is installed.
        let _ = task_exists();
    }

    #[test]
    fn current_user_sid_is_well_formed() {
        let sid = token::current_user_sid().unwrap();
        assert!(is_sid(&sid), "{sid}");
        assert!(task_sddl(&sid).is_some());
    }
}
