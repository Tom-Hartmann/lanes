//! Copies the FFmpeg DLLs next to the binaries cargo produces (including test
//! executables in `deps/`), so `cargo test` and `tauri dev` run without
//! touching PATH. Release bundles ship them via tauri.conf.json resources.

use std::path::PathBuf;
use std::{env, fs};

fn main() {
    let Some(ffmpeg) = env::var_os("FFMPEG_DIR").map(PathBuf::from) else { return };
    let bin = ffmpeg.join("bin");
    println!("cargo:rerun-if-changed={}", bin.display());
    // OUT_DIR = target/<profile>/build/<pkg>-<hash>/out
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let Some(profile_dir) = out.ancestors().nth(3) else { return };
    let Ok(entries) = fs::read_dir(&bin) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")) {
            for dir in [profile_dir.to_path_buf(), profile_dir.join("deps")] {
                let _ = fs::create_dir_all(&dir);
                let dst = dir.join(entry.file_name());
                let stale = fs::metadata(&dst).map(|m| m.len()).ok() != fs::metadata(&path).map(|m| m.len()).ok();
                if stale {
                    let _ = fs::copy(&path, &dst);
                }
            }
        }
    }
}
