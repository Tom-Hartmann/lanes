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

/// `dir/name.ext`, adding `_2`, `_3`... if the file already exists.
pub fn unique_path(dir: &Path, name: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{name}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2..).map(|n| dir.join(format!("{name}_{n}.{ext}"))).find(|p| !p.exists()).expect("unbounded")
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
        let a = unique_path(&dir, "x", "mp4");
        std::fs::write(&a, b"").unwrap();
        let b = unique_path(&dir, "x", "mp4");
        assert_eq!(b.file_name().unwrap(), "x_2.mp4");
        let _ = std::fs::remove_dir_all(dir);
    }
}
