//! Pure path comparisons used by the autostart and admin checks.
//!
//! Windows paths are compared component by component and case-insensitively,
//! so `C:\Program Files\Lanes` matches `c:\program files\lanes` but
//! `C:\Program Files Evil\lanes.exe` does not count as being inside
//! `C:\Program Files`. Callers canonicalize first (`std::fs::canonicalize`),
//! which resolves links, 8.3 names and trailing dots; these helpers only
//! normalize spelling and refuse anything that still contains `.` or `..`.

use std::path::{Component, Path, PathBuf, Prefix};

/// Splits an absolute path into lowercase components with the drive or UNC
/// share folded into one leading element. Returns `None` for relative paths,
/// `..`/`.` components or names that are not valid Unicode.
fn normalized(path: &Path) -> Option<Vec<String>> {
    if !path.is_absolute() {
        return None;
    }
    let mut out = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => out.push(match prefix.kind() {
                // `\\?\C:\` (what canonicalize returns) and `C:\` name the same volume.
                Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                    format!("{}:", char::from(letter).to_ascii_uppercase())
                }
                Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => {
                    format!(r"\\{}\{}", server.to_str()?.to_lowercase(), share.to_str()?.to_lowercase())
                }
                _ => prefix.as_os_str().to_str()?.to_lowercase(),
            }),
            Component::RootDir => {}
            Component::CurDir | Component::ParentDir => return None,
            Component::Normal(name) => out.push(name.to_str()?.to_lowercase()),
        }
    }
    Some(out)
}

/// True if `path` lies strictly below `root` (a path is not "within" itself).
pub fn is_within(path: &Path, root: &Path) -> bool {
    match (normalized(path), normalized(root)) {
        (Some(path), Some(root)) => path.len() > root.len() && path.starts_with(&root),
        _ => false,
    }
}

/// True if both paths name the same location after normalization.
pub fn same_path(a: &Path, b: &Path) -> bool {
    match (normalized(a), normalized(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Removes the `\\?\` prefix `canonicalize` adds, for paths handed to other
/// programs (Task Scheduler, the Run key) that may not accept verbatim paths.
pub fn strip_verbatim(path: &Path) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path.to_path_buf();
    };
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    match text.strip_prefix(r"\\?\") {
        // Only drive-letter paths have a plain form; `\\?\Volume{..}` stays as it is.
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> &Path {
        Path::new(s)
    }

    #[test]
    fn within_is_component_aware_and_case_insensitive() {
        assert!(is_within(p(r"C:\Program Files\Lanes\lanes.exe"), p(r"C:\Program Files")));
        assert!(is_within(p(r"c:\program files\lanes\lanes.exe"), p(r"C:\Program Files")));
        assert!(is_within(p(r"\\?\C:\Program Files\Lanes\lanes.exe"), p(r"C:\Program Files")));
        assert!(is_within(p(r"C:\Program Files\Lanes\lanes.exe"), p(r"\\?\C:\Program Files\")));
        // Sibling directory sharing a string prefix.
        assert!(!is_within(p(r"C:\Program Files Evil\lanes.exe"), p(r"C:\Program Files")));
        assert!(!is_within(p(r"C:\Program FilesX\lanes.exe"), p(r"C:\Program Files")));
        // The root itself, other drives, relative paths.
        assert!(!is_within(p(r"C:\Program Files"), p(r"C:\Program Files")));
        assert!(!is_within(p(r"D:\Program Files\Lanes\lanes.exe"), p(r"C:\Program Files")));
        assert!(!is_within(p(r"Program Files\Lanes\lanes.exe"), p(r"C:\Program Files")));
    }

    #[test]
    fn within_rejects_dot_dot() {
        assert!(!is_within(p(r"C:\Program Files\..\Users\x\lanes.exe"), p(r"C:\Program Files")));
        assert!(!is_within(p(r"C:\Program Files\Lanes\..\..\lanes.exe"), p(r"C:\Program Files")));
    }

    #[test]
    fn within_handles_unc() {
        assert!(is_within(p(r"\\Server\Share\dir\a.exe"), p(r"\\server\share\DIR")));
        assert!(is_within(p(r"\\?\UNC\server\share\dir\a.exe"), p(r"\\server\share")));
        assert!(!is_within(p(r"\\server\other\a.exe"), p(r"\\server\share")));
    }

    #[test]
    fn same_path_ignores_case_and_verbatim_prefix() {
        assert!(same_path(p(r"\\?\C:\Program Files\Lanes\lanes.exe"), p(r"c:\program files\lanes\LANES.EXE")));
        assert!(!same_path(p(r"C:\a\lanes.exe"), p(r"C:\b\lanes.exe")));
        assert!(!same_path(p(r"C:\a\..\a\lanes.exe"), p(r"C:\a\lanes.exe")));
    }

    #[test]
    fn strips_verbatim_prefixes() {
        assert_eq!(strip_verbatim(p(r"\\?\C:\Program Files\Lanes\lanes.exe")), p(r"C:\Program Files\Lanes\lanes.exe"));
        assert_eq!(strip_verbatim(p(r"\\?\UNC\server\share\a.exe")), p(r"\\server\share\a.exe"));
        assert_eq!(strip_verbatim(p(r"C:\x\a.exe")), p(r"C:\x\a.exe"));
        // Volume GUID paths have no drive-letter form; keep them as they are.
        let guid = r"\\?\Volume{01234567-89ab-cdef-0123-456789abcdef}\a.exe";
        assert_eq!(strip_verbatim(p(guid)), p(guid));
    }
}
