# Testing

Manual test status. Tick a box when a feature has been tried on real hardware and worked; add a
note (machine, date, what broke) when it did not. Automated tests: `cargo test --workspace`, plus
`cargo test -p lanes-core --test live_session -- --ignored --nocapture` for a real capture run.

Hardware so far: NVIDIA RTX 5060 Ti, Windows 11, 1 and 3 monitors (two of the same model),
SteelSeries Sonar audio devices, Blade & Soul NEO in borderless fullscreen.

Automated full test, 2026-10-20 (release build, sandboxed settings per case, driven by the
global hotkeys): follow game, window mode, HEVC/AV1/x264, Opus, four-lane per-app audio, disk and
RAM-capped replay, clip presets, canvas and separate-file multi-monitor with caps, pause/resume,
disk guard, crash safety, a 5-minute recording, clips library playback and remux. Measured
0.2–0.7 % CPU while capturing (1.7 % with x264).

## Done

- [x] Replay buffer: start/stop from the Dashboard button
- [x] Save clip from the Dashboard preset buttons
- [x] Clips library: thumbnails, opening and playing clips
- [x] Monitors: 1 screen
- [x] Monitors: 3 screens, one file per monitor
- [x] Monitors: 3 screens, combined canvas

## Capture

- [x] Follow game: captures the detected game's window (borderless, unfocused) — switching back on close not yet seen
- [x] Window mode: window picked by exe (resizing / alt-tab not yet tested)
- [x] Per-monitor caps (e.g. secondary at 30 fps / 720p) in both multi-monitor modes
- [x] Output resolution presets (1080p / 720p / custom) and frame rates other than 60 (720p30 tested)
- [ ] Capture method forced to Desktop Duplication (no cursor is expected)
- [ ] Capture cursor on/off
- [ ] A game that blocks capture (anti-cheat / black frames): fallback and error notice
- [ ] Resolution change and alt-tab in exclusive fullscreen while recording

## Encoding

- [x] NVENC HEVC and AV1
- [ ] AMD AMF (AMD GPU as the main GPU)
- [ ] Intel QuickSync (Intel GPU as the main GPU)
- [x] x264 fallback (choose it in Settings)
- [ ] CBR, VBR and CQP; Performance / Balanced / Quality presets
- [x] Opus audio

## Recording

- [x] Start/stop via hotkey (Ctrl+Shift+F9) — tray not yet tested
- [x] Pause/resume (Ctrl+Shift+F8): no gap or jump in the file
- [x] MP4 remux on stop (keeping MKV with remux off not yet tested)
- [x] Filename template with {game} {preset} {date} {time}
- [x] Disk-space guard: stop mode refuses to start (warn mode not yet tested)
- [x] Long recording: 5 min, audio/video end within a few ms (1 h+ not yet tested)
- [x] App killed mid-recording: the MKV is still playable and converts to MP4

## Replay buffer & clips

- [x] Save clip via hotkeys (Ctrl+Shift+F10 / F12) — with the game unfocused; in-game focus not yet tested
- [x] Toggle replay via hotkey (Ctrl+Shift+F11) — tray not yet tested
- [ ] Clip sound and Windows notification (game keeps focus)
- [x] Preset with a lane subset, own folder and own filename template
- [x] Quality preset Medium (background re-encode on NVENC)
- [x] Disk-backed replay buffer
- [x] RAM cap: buffer stops growing at the cap
- [x] Clip longer than the buffer holds (gets what is there)
- [x] Recording and replay buffer at the same time

## Audio lanes

- [x] Both default tracks (Game, Mic) present and named in the file
- [x] Per-app source on its own lane (test tone via ffplay.exe; Discord/game captured but were silent)
- [ ] One source routed to several lanes
- [ ] Volume, mute and mono, per lane and per source, changed live while recording
- [ ] Noise gate and noise suppression on a mic
- [ ] "Split voices" button
- [ ] Drag & drop and the matrix view
- [ ] Meters move with sound
- [ ] Adding a lane applies to the next recording
- [ ] A/V sync by ear/eye (clap test) — timestamps verified: audio within 20 ms at start and end

## Devices

- [ ] Unplug and replug the mic while recording: notice, then audio resumes
- [ ] Change the Windows default output/input while recording: Lanes follows it
- [ ] Specific (non-default) devices selected

## Games & profiles

- [ ] Game from the game list detected; Dashboard shows it
- [x] Unknown fullscreen game detected by the heuristic, also when not focused
- [ ] Profile switches automatically when the game starts and back when it exits
- [ ] Create, duplicate, rename, delete profiles; per-profile replay length

## Clips library

- [ ] Trim (lossless, snaps to keyframes)
- [x] Convert to MP4 (MKV clips), and MKV playback in the viewer
- [ ] Delete goes to the recycle bin
- [ ] Show in folder / open output folder
- [ ] Search and the clip/recording filter

## Hotkeys

- [ ] Rebinding in the Hotkeys page
- [ ] Hotkeys inside a fullscreen game
- [ ] Combo already owned by another app (e.g. ShadowPlay): falls back to keyboard hook
- [ ] "Keyboard hook" method: game still receives the key

## Admin mode & autostart

- [ ] Start with Windows (normal mode): starts minimized after the delay, optional replay/profile
- [ ] Installer: installs to Program Files, uninstall removes everything
- [ ] Code signing with the dev certificate (`scripts/new-dev-cert.ps1`, `docs/SIGNING.md`)
- [ ] Admin mode on: one UAC prompt, badge shows Admin after restart
- [ ] Admin mode + Start with Windows: starts elevated at logon without a prompt
- [ ] Tray "Restart as administrator": no UAC prompt
- [ ] Admin mode off / uninstall: the scheduled task is removed
- [ ] Hotkeys and per-app audio with a game running as administrator

## Installer & releases

- [x] Portable ZIP: unzipped to a fresh folder, starts and captures (Lanes.exe + 4 FFmpeg DLLs)
- [x] lanes.exe needs no Visual C++ runtime (static CRT; only Windows DLLs + FFmpeg)
- [ ] Installer on this PC: installs to Program Files, Start menu entry, starts, uninstall cleans up
- [ ] Installer on a clean PC without dev tools (WebView2 bootstrapper if missing)
- [ ] Portable ZIP on a clean PC
- [x] GitHub "Release" workflow: tag `vX.Y.Z` publishes installer + portable ZIP (v0.1.0)
- [x] GitHub "CI" workflow: clippy, tests and UI typecheck pass on windows-latest
- [ ] Signed release build (`LANES_SIGN_*` secrets, see docs/SIGNING.md)

## Performance

- [ ] PresentMon frame time with and without Lanes in a real game (`docs/PERFORMANCE.md`)
- [ ] CPU / RAM while recording at 1440p and 4K
- [x] UI closed: no WebView2 processes left (Task Manager)
