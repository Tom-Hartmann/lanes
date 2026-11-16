//! Output file naming from templates such as `{game}_{date}_{time}`.

use std::path::{Path, PathBuf};

use windows::Win32::System::SystemInformation::GetLocalTime;

/// Values substituted into a filename template.
#[derive(Clone, Debug, Default)]
pub struct NameParts {
    pub game: String,
    pub profile: String,
    pub preset: String,
    pub date: String,
    pub time: String,
}

impl NameParts {
    /// Fills date and time from the local clock.
    pub fn now(game: &str, profile: &str, preset: &str) -> Self {
        // SAFETY: no arguments.
        let t = unsafe { GetLocalTime() };
        Self {
            game: game.into(),
            profile: profile.into(),
            preset: preset.into(),
            date: format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay),
            time: format!("{:02}-{:02}-{:02}", t.wHour, t.wMinute, t.wSecond),
        }
    }
}

/// Renders `template` and makes it safe as a Windows file name (no extension).
pub fn render(template: &str, parts: &NameParts) -> String {
    let game = if parts.game.is_empty() { "Desktop" } else { &parts.game };
    let raw = template
        .replace("{game}", game)
        .replace("{profile}", &parts.profile)
        .replace("{preset}", &parts.preset)
        .replace("{date}", &parts.date)
        .replace("{time}", &parts.time);
    sanitize(&raw)
}

/// Replaces characters Windows forbids, trims trailing dots/spaces and
/// avoids reserved device names.
pub fn sanitize(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .collect();
    while s.ends_with(['.', ' ']) {
        s.pop();
    }
    let s = s.trim_start().to_string();
    let upper = s.to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str())
        || ((upper.starts_with("COM") || upper.starts_with("LPT")) && upper.len() == 4 && upper.as_bytes()[3].is_ascii_digit());
    if s.is_empty() {
        "Lanes".into()
    } else if reserved {
        format!("_{s}")
    } else {
        s.chars().take(150).collect()
    }
}

/// A cross-thread/process reservation, held until the writer finishes.
#[derive(Clone, Debug)]
pub struct ReservedPath(std::sync::Arc<Reservation>);

#[derive(Debug)]
struct Reservation { path: PathBuf, lock: PathBuf }

impl Drop for Reservation {
    fn drop(&mut self) { let _ = std::fs::remove_file(&self.lock); }
}

impl std::ops::Deref for ReservedPath {
    type Target = Path;
    fn deref(&self) -> &Path { &self.0.path }
}

/// Atomically reserves `dir/name.ext`, adding a counter on collisions.
pub fn unique_path(dir: &Path, name: &str, ext: &str) -> std::io::Result<ReservedPath> {
    std::fs::create_dir_all(dir)?;
    for n in 1u64.. {
        let stem = if n == 1 { name.to_owned() } else { format!("{name}_{n}") };
        let path = dir.join(format!("{stem}.{ext}"));
        let lock = dir.join(format!("{stem}.{ext}.reserve"));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&lock) {
            Ok(_) => {
                let reserved = ReservedPath(std::sync::Arc::new(Reservation { path, lock }));
                if !reserved.exists() { return Ok(reserved); }
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("filename space exhausted"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts() -> NameParts {
        NameParts { game: "Elden Ring".into(), profile: "Default".into(), preset: "Last 30s".into(), date: "2026-10-20".into(), time: "21-05-09".into() }
    }

    #[test]
    fn renders_placeholders() {
        assert_eq!(render("{game}_{date}_{time}", &parts()), "Elden Ring_2026-10-20_21-05-09");
        assert_eq!(render("{profile}-{preset}", &parts()), "Default-Last 30s");
    }

    #[test]
    fn missing_game_becomes_desktop() {
        let p = NameParts { game: String::new(), ..parts() };
        assert_eq!(render("{game}", &p), "Desktop");
    }

    #[test]
    fn sanitizes_forbidden_names() {
        assert_eq!(sanitize(r#"a<b>c:d"e/f\g|h?i*"#), "a_b_c_d_e_f_g_h_i_");
        assert_eq!(sanitize("clip. . "), "clip");
        assert_eq!(sanitize("CON"), "_CON");
        assert_eq!(sanitize("com1"), "_com1");
        assert_eq!(sanitize("   "), "Lanes");
        assert_eq!(sanitize("Game: The Sequel"), "Game_ The Sequel");
    }

    #[test]
    fn unique_path_appends_counter() {
        let dir = std::env::temp_dir().join(format!("lanes-naming-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = unique_path(&dir, "x", "mp4").unwrap();
        // Even before a writer creates the output, the name is reserved.
        let b = unique_path(&dir, "x", "mp4").unwrap();
        assert_eq!(b.file_name().unwrap(), "x_2.mp4");
        drop(a);
        let c = unique_path(&dir, "x", "mp4").unwrap();
        assert_eq!(c.file_name().unwrap(), "x.mp4");
        drop((b, c));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn simultaneous_writers_reserve_different_paths() {
        let dir = crate::files::WorkDir::new(&std::env::temp_dir()).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8).map(|_| {
            let dir = dir.path().to_owned();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                unique_path(&dir, "same-second", "mp4").unwrap()
            })
        }).collect();
        let reservations: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        let distinct: std::collections::HashSet<_> = reservations.iter().map(|p| p.to_path_buf()).collect();
        assert_eq!(distinct.len(), 8);
        drop(reservations);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
