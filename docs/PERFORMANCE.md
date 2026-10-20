# Performance

Lanes' first design goal is that you barely notice it running. This page lists
what the design does for that, what has been measured, and how to measure the
in-game frame-time impact yourself.

## How the pipeline stays cheap

| Stage | Thread | What happens | Cost |
| --- | --- | --- | --- |
| Capture | WGC thread pool (or one Desktop Duplication thread) | The newest frame stays a GPU texture in a one-slot mailbox | No copy, no CPU readback |
| Convert/scale | Video thread per output stream (above normal) | D3D11 video processor: BGRA → NV12 BT.709, scale + letterbox (or a canvas of several monitors) | Fixed-function video engine, not the 3D queue the game uses |
| Encode | Same video thread | NVENC/AMF/QSV read the NV12 surface straight from an FFmpeg D3D11 frames pool | Zero-copy; x264 is a fallback with readback |
| Pacing | Same video thread | High-resolution waitable timer; CFR with clean duplicate/drop counting | Sleeps between frames |
| Audio | One MMCSS "Pro Audio" thread per source, mixer thread, encoder thread | Event-driven WASAPI, QPC-stamped; 10 ms blocks | Fractions of a percent |
| Routing | Router thread | Encoded packets go to the replay buffer (RAM or disk segments) and the recorder | Disk I/O never on capture/encode threads |
| UI | WebView2 (below-normal priority) | Only exists while the window is open; meters 20 Hz and stats 1 Hz are pushed, never polled | 0 when closed |

Idle (no recording, no replay buffer) there is no capture session at all: no
GPU device, no encoder, no audio clients. Only the hotkey thread (blocked in
`GetMessage`) and the game detector (one process snapshot every 2 s) exist.

## Measured (RTX 5060 Ti, Windows 11 26200, release build, UI closed)

| State | CPU (whole machine) | Private memory |
| --- | --- | --- |
| Idle in tray | 0.02 % | 24 MB |
| 1080p60 NVENC H.264, replay buffer + recording, 2 audio lanes | 0.08 % | 209 MB (≈ 60 MB of that is replay buffer) |
| Idle again after the session | 0.02 % | 29 MB |

Encoder: 1.2–2 ms per frame (≈ 7–12 % of the 16.7 ms frame budget), 0 dropped
frames, A/V offset at file start ≤ 20 ms (one AAC frame).

## Measuring in-game frame time (PresentMon)

The `< 1 ms added frame time` target can only be checked with a real game:

1. Install [PresentMon](https://github.com/GameTechDev/PresentMon) (the console
   build, `PresentMon-x64.exe`).
2. Pick a repeatable scene (benchmark mode or standing still in a menu-free spot).
   Lock the frame rate *off* (or well above the monitor rate) so changes show up.
3. Baseline, Lanes not running:
   `PresentMon-x64.exe --process_name game.exe --timed 60 --output_file base.csv`
4. With Lanes capturing (replay buffer on, recording on):
   `PresentMon-x64.exe --process_name game.exe --timed 60 --output_file lanes.csv`
5. Compare the average and 99th-percentile of `MsBetweenPresents` (or
   `FrameTime` in newer versions). The difference is the added frame time.
6. For GPU impact, compare `GPU Engine` usage in Task Manager or use
   `typeperf "\GPU Engine(*)\Utilization Percentage"`: the capture/convert work
   shows on the *Video Processing* engine and encoding on *Video Encode*, not on
   *3D*.

Things that raise the cost, in order: x264 (CPU encoding), the Quality encoder
preset, HEVC/AV1 at very high resolutions, several separate monitor streams at
full frame rate, the disk-backed replay buffer on a slow HDD.
