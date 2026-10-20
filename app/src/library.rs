//! The clips library: files in the output folders, plus a small index
//! (`%APPDATA%\Lanes\library.json`) remembering which files are clips and
//! their durations, so listing does not re-open every file.

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

    pub fn add(&self, path: &Path, kind: Kind) {
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        index.insert(key(path), Entry { kind, duration: lanes_core::media::duration(path), size });
        self.save(&index);
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
                    Entry { kind: Kind::Recording, duration: None, size: 0 }
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
        lib.add(&dir.join("a.mp4"), Kind::Clip);
        let items = lib.list(std::slice::from_ref(&dir));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, Kind::Clip);
        assert!(is_within(&dir.join("a.mp4"), std::slice::from_ref(&dir)));
        assert!(!is_within(&dir.join("..").join("x.mp4"), std::slice::from_ref(&dir)));
        let _ = fs::remove_dir_all(dir);
    }
}
