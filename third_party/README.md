# third_party

Native dependencies that are downloaded, not committed.

- `ffmpeg/`: FFmpeg 9.0 shared build (headers, import libs, DLLs). Fetch it with
  `powershell -File scripts/fetch-ffmpeg.ps1`. `.cargo/config.toml` points `FFMPEG_DIR` here,
  `core/build.rs` copies the DLLs next to built binaries, and `tauri.conf.json` bundles them.
