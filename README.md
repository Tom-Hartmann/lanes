# Lanes

Lightweight game capture and clipping for Windows 10/11, with every audio source on its own
lane. Rust backend, React UI, Tauri v2.

**Download:** [latest release](../../releases/latest): `Lanes_*_x64-setup.exe` (installer) or
`Lanes_*_x64-portable.zip` (no install). Windows 10/11 x64. Test status per feature:
[TESTING.md](TESTING.md).

## Features

### Capture
Records your game or desktop straight from the GPU, so the game barely notices.
- One or more monitors, a single window, or follow the detected game
- Windows.Graphics.Capture, with Desktop Duplication as a fallback
- Scaling and colour conversion on the GPU's video engine, not the 3D engine your game uses

### Encoding
Hardware first, CPU only as a fallback.
- NVIDIA NVENC, AMD AMF, Intel QuickSync, or x264 on the CPU
- H.264, HEVC and AV1 when your GPU supports them
- CBR, VBR or constant quality; AAC or Opus audio
- Picks the best encoder automatically

### Recording
Only records when you tell it to.
- Start, stop and pause from a hotkey, the tray or the app
- Crash-safe MKV while recording, converted to MP4 when you stop
- Filename templates like `{game}_{date}_{time}`
- Disk-space guard warns you or stops recording when space runs low

### Replay buffer & clips
Save the play that just happened.
- 15 s to 10 min of history, in RAM or on disk
- Instant clips that always start cleanly on a keyframe
- Several clip hotkeys, e.g. 30 s and 2 min
- Per preset: lanes, quality, folder and filename
- Feedback on save that never steals focus: a chime on the output device you pick, an on-screen notice in a corner with your own text, and/or a Windows notification, each with a Test button

### Audio lanes
Every source on its own track, so you can edit them separately later.
- Name your lanes: Game, Discord, Mic, Music…
- Sources: system audio, a single app, or any microphone
- A source can feed several lanes
- Volume, mute and mono per lane and per source
- Noise gate and noise suppression for mics
- One click puts each mic and voice app on its own lane
- Drag & drop or a sources × lanes grid
- Live level meters

### Devices
- Pick any mic or output device, or follow the Windows default
- Survives unplugging and default-device changes, with a notice instead of silent audio

### Games & profiles
- Detects games from your list or because they run fullscreen
- Per-game profiles switch automatically: capture, lanes, fps, buffer length

### Multi-monitor
- Monitor list with name, resolution, refresh rate and a preview
- One file per monitor, or all combined side by side
- Per-monitor caps, e.g. your second screen at 30 fps / 720p

### Clips library
- Thumbnails, search and filters
- Play, trim losslessly, convert to MP4
- Share: one-click copy with your lanes mixed into one track and sized for Discord; drag it out, copy it (Ctrl+V in Discord) or send it to a Discord channel via webhook
- Delete to the recycle bin, or show the file in its folder

### Hotkeys
- Global hotkeys that work while a game has focus
- Rebind by pressing the new combo
- Falls back to a keyboard hook automatically if another app owns a combo

### Admin mode & autostart
- Start with Windows: delay, minimized, start the replay buffer, load a profile
- Optional admin mode for games running as administrator: one UAC prompt to set up, then no more prompts
- An Admin / Standard badge shows which mode Lanes is in

### Lightweight
- Idle in the tray: ~0 % CPU, 24 MB RAM
- Recording 1080p60: under 0.1 % CPU
- Closing the window fully shuts the UI down
- Live stats: fps, dropped frames, encoder load, buffer size, disk speed

## Layout

| Path        | What                                                                                 |
| ----------- | ------------------------------------------------------------------------------------ |
| `core/`     | Engine: capture, convert, encode, audio, mux, replay buffer, media tools. No Tauri.   |
| `platform/` | Windows integration: autostart (Run key), elevated scheduled task, signature checks. |
| `app/`      | Tauri shell: commands/events, settings, tray, hotkeys, game detection, clips library. |
| `ui/`       | React + TypeScript + Vite + Zustand + Tailwind. `ui/src/lib/ipc.ts` is the contract. |
| `scripts/`  | FFmpeg download, code signing, dev certificate.                                       |
| `docs/`     | `PERFORMANCE.md`, `SIGNING.md`.                                                       |

## Build

Requirements: Rust stable (MSVC), Node 20+, pnpm, LLVM/clang (for the FFmpeg bindings; `LIBCLANG_PATH`
is set in `.cargo/config.toml`, adjust if LLVM is not in `C:\Program Files\LLVM`).

```sh
powershell -File scripts/fetch-ffmpeg.ps1   # FFmpeg 9.0 shared (GPL: includes x264) into third_party/
pnpm install
pnpm dev                                    # tauri dev
cargo test --workspace                      # unit tests (no hardware needed)
cargo test -p lanes-core --test live_session -- --ignored --nocapture   # real capture/encode test
pnpm build                                  # release + per-machine NSIS installer
```

`pnpm package:portable` then packs `target/release/lanes.exe` and the FFmpeg DLLs into
`target/release/bundle/portable/*.zip`. The installer is unsigned until you configure signing
(`docs/SIGNING.md`); the build prints a warning for each unsigned file.

## Releases (GitHub Actions)

- **CI** (`.github/workflows/ci.yml`): every push to `main` and every pull request runs clippy,
  the tests and the UI typecheck on Windows.
- **Release** (`.github/workflows/release.yml`): pushing a tag builds the installer and the portable
  ZIP and publishes them as a GitHub Release:

  ```sh
  git tag v0.2.0
  git push origin v0.2.0
  ```

  The version is taken from the tag. "Run workflow" on the Actions tab builds the same files
  without publishing (they appear as build artifacts). Repository variable `FFMPEG_LICENSE`
  (`gpl` default, or `lgpl`) picks the FFmpeg build; secrets `LANES_SIGN_*` enable code signing.

## Runtime model

- **Session**: a GPU device, video streams, audio session and packet router exist only while recording
  or the replay buffer is on. Idle, Lanes is a tray icon, a blocked hotkey thread and a 2-second game
  check.
- **One clock**: video frames, audio packets and hotkey presses are stamped with QPC. Video is CFR,
  frame *n* at `t0 + n/fps`; audio samples are placed on the same timeline, so A/V sync holds without
  drift correction. Clips and recordings are cut by timestamp, not arrival order (encoders deliver with
  different latencies).
- **Encode once, use twice**: the router fans each packet out to the recorder (MKV while recording,
  crash-safe; optional MP4 remux when done) and the replay buffer.
- **Recovery**: device loss or a failed stream rebuilds the session and continues recording into a new
  file; audio devices that vanish or change are reopened (default devices are followed); a closed game
  window falls back to the monitor. Errors reach the UI as `engine_error` events, never panics.
- **Single instance** per session (named mutex shared by elevated and standard instances). Starting
  `lanes.exe` again shows the window.

## Admin mode

Off by default; the app runs `asInvoker`. Enabling it in Settings shows one UAC prompt, which registers
a Task Scheduler task (COM API) that starts Lanes elevated at logon (if "Start with Windows" is on) and
on demand from the tray ("Restart as administrator") without further prompts. The task only points to a
signed exe under Program Files. Disabling it, or uninstalling, removes the task. See
`platform/src/admin` and `docs/SIGNING.md`.

## Known limitations

- **Anti-cheat / protected content**: some games (and DRM video) block WGC capture or show black frames.
  Lanes falls back to Desktop Duplication for monitors, which some anti-cheats also block. There is no
  injection-based capture by design (see `docs/SIGNING.md` on Defender hygiene).
- **Elevated games**: a standard-user Lanes cannot hook keys or capture per-app audio of games running
  as administrator (UIPI). `RegisterHotKey` hotkeys still work. Admin mode fixes this.
- **Per-application audio** needs Windows 10 2004 (build 19041); the UI disables it on older builds.
- **Desktop Duplication fallback** does not draw the mouse cursor.
- **Hook-mode hotkeys** do not fire while the Lanes window itself has focus (registered hotkeys do).
- **Lane changes** (adding/removing lanes) and video/encoder changes apply to the next session;
  volumes, mutes, routing and sources apply live.
- **Clip quality presets** other than "Original" re-encode the saved clip in the background.
- Only tested on NVIDIA so far: AMF and QSV, admin mode (UAC + task) and the installer flow have
  not been run. See [TESTING.md](TESTING.md).
