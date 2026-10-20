//! Per-user autostart through the `Run` key.
//!
//! The value lives under `HKCU`, so it needs no elevation and only affects the
//! current user. It starts the *non-elevated* app; elevated autostart is the
//! scheduled task in [`crate::admin`].

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use crate::{paths, sys};

/// `HKEY_CURRENT_USER\<RUN_KEY>` holds one value per autostart program.
pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Name of our value under [`RUN_KEY`].
pub const VALUE_NAME: &str = "Lanes";
/// Argument appended to the command so the app knows it was started at logon.
pub const AUTOSTART_ARG: &str = "--autostart";

/// Adds or removes the `Run` value for the current executable.
/// Removing a value that does not exist succeeds.
pub fn set_enabled(enabled: bool) -> io::Result<()> {
    if enabled {
        let exe = paths::strip_verbatim(&std::env::current_exe()?);
        sys::registry::set_string(RUN_KEY, VALUE_NAME, &run_command(&exe))
    } else {
        sys::registry::delete_value(RUN_KEY, VALUE_NAME)
    }
}

/// True if the `Run` value exists and starts the executable that is running now.
/// A value left behind by another copy (an old install path, a dev build) counts as disabled.
pub fn is_enabled() -> bool {
    let command = match sys::registry::get_string(RUN_KEY, VALUE_NAME) {
        Ok(Some(command)) => command,
        Ok(None) => return false,
        Err(e) => {
            log::warn!("reading autostart value: {e}");
            return false;
        }
    };
    let (Some(configured), Ok(current)) = (command_exe(&command), std::env::current_exe()) else {
        return false;
    };
    same_file(&configured, &current)
}

/// Compares two executable paths, resolving links and 8.3 names where the files exist.
fn same_file(a: &Path, b: &Path) -> bool {
    let a = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let b = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    paths::same_path(&a, &b)
}

/// Builds the `Run` command: the quoted executable path followed by [`AUTOSTART_ARG`].
/// Windows paths cannot contain `"`, so quoting needs no escaping.
pub(crate) fn run_command(exe: &Path) -> OsString {
    let mut command = OsString::from("\"");
    command.push(exe.as_os_str());
    command.push("\" ");
    command.push(AUTOSTART_ARG);
    command
}

/// Extracts the executable from a `Run` command: the quoted first token, or
/// everything up to the first space when unquoted.
pub(crate) fn command_exe(command: &OsStr) -> Option<PathBuf> {
    const QUOTE: u16 = b'"' as u16;
    const SPACE: u16 = b' ' as u16;
    let wide: Vec<u16> = command.encode_wide().collect();
    let start = wide.iter().position(|&c| c != SPACE)?;
    let trimmed = &wide[start..];
    let exe = if let Some(rest) = trimmed.strip_prefix(&[QUOTE]) {
        let end = rest.iter().position(|&c| c == QUOTE)?;
        &rest[..end]
    } else {
        let end = trimmed.iter().position(|&c| c == SPACE).unwrap_or(trimmed.len());
        &trimmed[..end]
    };
    (!exe.is_empty()).then(|| PathBuf::from(OsString::from_wide(exe)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_quotes_the_path() {
        let command = run_command(Path::new(r"C:\Program Files\Lanes\lanes.exe"));
        assert_eq!(command, OsStr::new(r#""C:\Program Files\Lanes\lanes.exe" --autostart"#));
    }

    #[test]
    fn command_round_trips() {
        let exe = Path::new(r"C:\Program Files\Lanes\lanes.exe");
        assert_eq!(command_exe(&run_command(exe)).as_deref(), Some(exe));
    }

    #[test]
    fn parses_unquoted_and_odd_commands() {
        assert_eq!(command_exe(OsStr::new(r"C:\x\lanes.exe --autostart")).as_deref(), Some(Path::new(r"C:\x\lanes.exe")));
        assert_eq!(command_exe(OsStr::new(r"  C:\x\lanes.exe")).as_deref(), Some(Path::new(r"C:\x\lanes.exe")));
        assert_eq!(command_exe(OsStr::new(r#""C:\no end quote"#)), None);
        assert_eq!(command_exe(OsStr::new(r#""" --autostart"#)), None);
        assert_eq!(command_exe(OsStr::new("   ")), None);
    }

    /// Touches the real `Run` key of the current user. Run explicitly with
    /// `cargo test -p lanes-platform -- --ignored autostart_round_trip`.
    #[test]
    #[ignore = "modifies HKCU\\...\\Run"]
    fn autostart_round_trip() {
        set_enabled(true).unwrap();
        assert!(is_enabled());
        set_enabled(false).unwrap();
        assert!(!is_enabled());
        set_enabled(false).unwrap();
    }
}
