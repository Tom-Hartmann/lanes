//! Private work directories and atomic file publication.
use std::{fs, io, path::{Path, PathBuf}, sync::atomic::{AtomicU64, Ordering}};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct WorkDir(PathBuf);

impl WorkDir {
    pub fn new(parent: &Path) -> io::Result<Self> {
        fs::create_dir_all(parent)?;
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        loop {
            let path = parent.join(format!("work-{}-{stamp}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }
    pub fn path(&self) -> &Path { &self.0 }
}

impl Drop for WorkDir {
    fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use io::Write;
    let work = WorkDir::new(path.parent().unwrap_or(Path::new(".")))?;
    let tmp = work.path().join("data");
    let mut file = fs::OpenOptions::new().create_new(true).write(true).open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(tmp, path)
}
