//! Copies the FFmpeg DLLs next to the test executables in `target/<profile>/deps`,
//! so `cargo test` runs without touching PATH. The app binary in
//! `target/<profile>` gets them from tauri-build (tauri.conf.json resources);
//! copying there too raced with tauri-build ("file in use") in parallel builds.

use std::path::PathBuf;
use std::{env, fs};

fn main() {
    let Some(ffmpeg) = env::var_os("FFMPEG_DIR").map(PathBuf::from) else { return };
    let bin = ffmpeg.join("bin");
    println!("cargo:rerun-if-changed={}", bin.display());
    // OUT_DIR = target/<profile>/build/<pkg>-<hash>/out
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let Some(profile_dir) = out.ancestors().nth(3) else { return };
    let deps = profile_dir.join("deps");
    let _ = fs::create_dir_all(&deps);
    let Ok(entries) = fs::read_dir(&bin) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")) {
            let dst = deps.join(entry.file_name());
            let stale = fs::metadata(&dst).map(|m| m.len()).ok() != fs::metadata(&path).map(|m| m.len()).ok();
            if stale {
                let _ = fs::copy(&path, &dst);
            }
        }
    }
}
