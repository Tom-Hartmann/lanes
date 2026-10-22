//! The clips library: files in the output folders, plus a small index
//! (`%APPDATA%\Lanes\library.json`) remembering which files are clips and
//! their durations, so listing does not re-open every file. It also keeps
//! per-file extras: the game, bookmarks and the viewer's saved audio mix.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Clip,
    Recording,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Entry {
    kind: Kind,
    duration: Option<f64>,
    size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    game: Option<String>,
    /// Bookmarks, seconds from the start.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    markers: Vec<f64>,
    /// The clip viewer's lane mix (opaque to the backend).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mix: Option<serde_json::Value>,
    /// The viewer mix rendered into the file's mixed track (`None`: every
    /// lane at 0 dB, as recorded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mix_baked: Option<serde_json::Value>,
}

impl Entry {
    fn new(kind: Kind) -> Self {
        Self { kind, duration: None, size: 0, game: None, markers: Vec::new(), mix: None, mix_baked: None }
    }
}

/// The game from a file name made by the default `{game}_{date}_{time}`
/// template, for files saved before the game was recorded in the index.
fn game_from_name(stem: &str) -> Option<String> {
    let b = stem.as_bytes();
    (0..b.len()).find_map(|i| {
        let date = b.get(i + 1..i + 12)?;
        let is_date = b[i] == b'_'
            && date.iter().enumerate().all(|(k, c)| match k {
                4 | 7 => *c == b'-',
                10 => *c == b'_',
                _ => c.is_ascii_digit(),
            });
        (is_date && i > 0).then(|| stem[..i].to_string())
    })
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipInfo {
    pub path: String,
    pub file_name: String,
    pub size_bytes: u64,
    pub created_ms: u64,
    pub duration_secs: Option<f64>,
    pub kind: Kind,
    /// "" when unknown; "Desktop" when no game was running.
    pub game: String,
    pub markers: Vec<f64>,
    pub mix: Option<serde_json::Value>,
    pub mix_baked: Option<serde_json::Value>,
}

pub struct Library {
    path: PathBuf,
    index: Mutex<HashMap<String, Entry>>,
}

const EXTENSIONS: &[&str] = &["mp4", "mkv", "mov"];

fn key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

impl Library {
    pub fn load(dir: &Path) -> Self {
        let path = dir.join("library.json");
        let index = fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Self { path, index: Mutex::new(index) }
    }

    fn save(&self, index: &HashMap<String, Entry>) {
        if let Some(dir) = self.path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(bytes) = serde_json::to_vec(index) {
            let _ = fs::write(&self.path, bytes);
        }
    }

    /// A file Lanes just saved, with the game ("" = none) and its bookmarks.
    pub fn add_saved(&self, path: &Path, kind: Kind, game: &str, markers: &[f64]) {
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let entry = Entry {
            duration: lanes_core::media::duration(path),
            size: fs::metadata(path).map(|m| m.len()).unwrap_or(0),
            game: Some(if game.is_empty() { "Desktop".into() } else { game.into() }),
            markers: markers.to_vec(),
            ..Entry::new(kind)
        };
        index.insert(key(path), entry);
        self.save(&index);
    }

    /// A copy of `src` (remux, trim). `range`: the trimmed part, so bookmarks
    /// outside it are dropped and the rest shifted.
    pub fn add_copy(&self, src: &Path, dst: &Path, kind: Kind, range: Option<(f64, f64)>) {
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let from = index.get(&key(src)).cloned();
        let (start, end) = range.unwrap_or((0.0, f64::MAX));
        let entry = Entry {
            duration: lanes_core::media::duration(dst),
            size: fs::metadata(dst).map(|m| m.len()).unwrap_or(0),
            game: from.as_ref().and_then(|e| e.game.clone()),
            markers: from
                .as_ref()
                .map(|e| e.markers.iter().filter(|&&m| m >= start && m <= end).map(|m| m - start).collect())
                .unwrap_or_default(),
            mix: from.as_ref().and_then(|e| e.mix.clone()),
            mix_baked: from.and_then(|e| e.mix_baked),
            ..Entry::new(kind)
        };
        index.insert(key(dst), entry);
        self.save(&index);
    }

    /// Stores (or clears) the viewer's audio mix for a file.
    pub fn set_mix(&self, path: &Path, mix: Option<serde_json::Value>) {
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        index.entry(key(path)).or_insert_with(|| Entry::new(Kind::Recording)).mix = mix;
        self.save(&index);
    }

    /// A copy whose mixed track was rendered from `mix` (the viewer's lane
    /// state): the viewer starts from it and knows the file already sounds so.
    pub fn set_baked_mix(&self, path: &Path, mix: Option<serde_json::Value>) {
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let entry = index.entry(key(path)).or_insert_with(|| Entry::new(Kind::Clip));
        entry.mix = mix.clone();
        entry.mix_baked = mix;
        self.save(&index);
    }

    pub fn kind(&self, path: &Path) -> Option<Kind> {
        self.index.lock().unwrap_or_else(|e| e.into_inner()).get(&key(path)).map(|e| e.kind)
    }

    pub fn remove(&self, path: &Path) {
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        if index.remove(&key(path)).is_some() {
            self.save(&index);
        }
    }

    /// All video files in `folders` (not recursive), newest first.
    pub fn list(&self, folders: &[PathBuf]) -> Vec<ClipInfo> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let mut dirty = false;
        for folder in folders {
            let Ok(entries) = fs::read_dir(folder) else { continue };
            for e in entries.flatten() {
                let path = e.path();
                let ext = path.extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase);
                if !ext.is_some_and(|x| EXTENSIONS.contains(&x.as_str())) || !seen.insert(key(&path)) {
                    continue;
                }
                let Ok(meta) = e.metadata() else { continue };
                let created = meta.created().or_else(|_| meta.modified()).ok();
                let entry = index.entry(key(&path)).or_insert_with(|| {
                    dirty = true;
                    Entry::new(Kind::Recording)
                });
                if entry.size != meta.len() {
                    // New or still being written: re-probe.
                    entry.duration = lanes_core::media::duration(&path);
                    entry.size = meta.len();
                    dirty = true;
                }
                out.push(ClipInfo {
                    path: path.to_string_lossy().into_owned(),
                    file_name: e.file_name().to_string_lossy().into_owned(),
                    size_bytes: meta.len(),
                    created_ms: created.and_then(|c| c.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64),
                    duration_secs: entry.duration,
                    kind: entry.kind,
                    game: entry
                        .game
                        .clone()
                        .or_else(|| game_from_name(&path.file_stem().unwrap_or_default().to_string_lossy()))
                        .unwrap_or_default(),
                    markers: entry.markers.clone(),
                    mix: entry.mix.clone(),
                    mix_baked: entry.mix_baked.clone(),
                });
            }
        }
        if dirty {
            self.save(&index);
        }
        out.sort_by_key(|c| std::cmp::Reverse(c.created_ms));
        out
    }
}

/// True if `path` is a file inside one of `roots` (after resolving `..` and
/// symlinks), so UI commands cannot touch arbitrary files.
pub fn is_within(path: &Path, roots: &[PathBuf]) -> bool {
    let Ok(p) = fs::canonicalize(path) else { return false };
    roots.iter().filter_map(|r| fs::canonicalize(r).ok()).any(|r| p.starts_with(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_videos_and_remembers_kind() {
        let dir = std::env::temp_dir().join(format!("lanes-lib-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.mp4"), b"x").unwrap();
        fs::write(dir.join("b.txt"), b"x").unwrap();
        let lib = Library::load(&dir.join("cfg"));
        lib.add_saved(&dir.join("a.mp4"), Kind::Clip, "Elden Ring", &[3.5]);
        lib.add_copy(&dir.join("a.mp4"), &dir.join("a_trim.mp4"), Kind::Clip, Some((2.0, 10.0)));
        let items = lib.list(std::slice::from_ref(&dir));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, Kind::Clip);
        assert_eq!(items[0].game, "Elden Ring");
        assert_eq!(items[0].markers, vec![3.5]);
        assert_eq!(lib.index.lock().unwrap()[&key(&dir.join("a_trim.mp4"))].markers, vec![1.5]);
        assert!(is_within(&dir.join("a.mp4"), std::slice::from_ref(&dir)));
        assert!(!is_within(&dir.join("..").join("x.mp4"), std::slice::from_ref(&dir)));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn game_from_default_template() {
        assert_eq!(game_from_name("Blade & Soul_2026-10-20_17-03-11").as_deref(), Some("Blade & Soul"));
        assert_eq!(game_from_name("Desktop_2026-10-20_17-03-11_trim").as_deref(), Some("Desktop"));
        assert_eq!(game_from_name("my clip"), None);
        assert_eq!(game_from_name("_2026-10-20_17"), None);
    }
}
