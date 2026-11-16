//! Cache identity and serialization for concurrent playback requests.
use std::{hash::{Hash, Hasher}, path::Path, sync::Mutex};

pub static WRITER: Mutex<()> = Mutex::new(());

pub fn key(path: &Path) -> std::io::Result<String> {
    let path = std::fs::canonicalize(path)?;
    let meta = std::fs::metadata(&path)?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    path.as_os_str().hash(&mut hash);
    meta.len().hash(&mut hash);
    meta.modified()?.hash(&mut hash);
    Ok(format!("v2-{:016x}", hash.finish()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn identical_names_and_sizes_in_different_folders_have_distinct_keys() {
        let dir = lanes_core::files::WorkDir::new(&std::env::temp_dir()).unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        std::fs::write(a.join("clip.mkv"), b"one").unwrap();
        std::fs::write(b.join("clip.mkv"), b"two").unwrap();
        assert_ne!(super::key(&a.join("clip.mkv")).unwrap(), super::key(&b.join("clip.mkv")).unwrap());
    }
}
