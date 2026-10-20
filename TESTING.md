# Testing

Manual test status. Tick a box when a feature has been tried on real hardware and worked; add a
note (machine, date, what broke) when it did not. Automated tests: `cargo test --workspace`, plus
`cargo test -p lanes-core --test live_session -- --ignored --nocapture` for a real capture run.

Hardware so far: NVIDIA RTX 5060 Ti, Windows 11, 1 and 3 monitors.

## Done

- [x] Replay buffer: start/stop from the Dashboard button
- [x] Save clip from the Dashboard preset buttons
- [x] Clips library: thumbnails, opening and playing clips
- [x] Monitors: 1 screen
- [x] Monitors: 3 screens, one file per monitor
- [x] Monitors: 3 screens, combined canvas

## Capture

- [ ] Follow game: capture switches to the game window when it starts, back to the monitor when it closes
- [ ] Window mode: pick a window by exe; window resizing and alt-tab
- [ ] Per-monitor caps (e.g. secondary at 30 fps / 720p) in both multi-monitor modes
- [ ] Output resolution presets (1080p / 720p / custom) and frame rates other than 60
- [ ] Capture method forced to Desktop Duplication (no cursor is expected)
- [ ] Capture cursor on/off
- [ ] A game that blocks capture (anti-cheat / black frames): fallback and error notice
- [ ] Resolution change and alt-tab in exclusive fullscreen while recording

## Encoding

- [ ] NVENC HEVC and AV1
- [ ] AMD AMF (AMD GPU as the main GPU)
- [ ] Intel QuickSync (Intel GPU as the main GPU)
- [ ] x264 fallback (choose it in Settings)
- [ ] CBR, VBR and CQP; Performance / Balanced / Quality presets
- [ ] Opus audio

## Recording

- [ ] Start/stop via hotkey (Ctrl+Shift+F9), tray and Dashboard
- [ ] Pause/resume (Ctrl+Shift+F8): no gap or jump in the file
- [ ] MP4 remux on stop, and keeping MKV with remux turned off
- [ ] Filename template with {game} {profile} {date} {time}
- [ ] Disk-space guard: warn and stop modes
- [ ] Long recording (1 h+): file plays, A/V still in sync at the end
- [ ] App killed mid-recording: the MKV is still playable

## Replay buffer & clips

- [ ] Save clip via hotkeys (Ctrl+Shift+F10 / F12) while a game has focus
- [ ] Toggle replay via hotkey (Ctrl+Shift+F11) and tray
- [ ] Clip sound and Windows notification (game keeps focus)
- [ ] Preset with a lane subset, own folder and own filename template
- [ ] Quality presets High / Medium / Low (background re-encode)
- [ ] Disk-backed replay buffer
- [ ] RAM cap: buffer stops growing at the cap
- [ ] Clip longer than the buffer holds (gets what is there)
- [ ] Recording and replay buffer at the same time

## Audio lanes

- [ ] Both default tracks (Game, Mic) present and named in the file (VLC/mpv track list)
- [ ] Per-app source (Spotify, browser, Discord) on its own lane
- [ ] One source routed to several lanes
- [ ] Volume, mute and mono, per lane and per source, changed live while recording
- [ ] Noise gate and noise suppression on a mic
- [ ] "Split voices" button
- [ ] Drag & drop and the matrix view
- [ ] Meters move with sound
- [ ] Adding a lane applies to the next recording
- [ ] A/V sync by ear/eye (clap test)

## Devices

- [ ] Unplug and replug the mic while recording: notice, then audio resumes
- [ ] Change the Windows default output/input while recording: Lanes follows it
- [ ] Specific (non-default) devices selected

## Games & profiles

- [ ] Game from the game list detected; Dashboard shows it
- [ ] Unknown fullscreen game detected by the heuristic (browsers/video players are not)
- [ ] Profile switches automatically when the game starts and back when it exits
- [ ] Create, duplicate, rename, delete profiles; per-profile replay length

## Clips library

- [ ] Trim (lossless, snaps to keyframes)
- [ ] Convert to MP4 (MKV clips)
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

## Performance

- [ ] PresentMon frame time with and without Lanes in a real game (`docs/PERFORMANCE.md`)
- [ ] CPU / RAM while recording at 1440p and 4K
- [ ] UI closed: no WebView2 processes left (Task Manager)
