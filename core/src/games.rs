//! Game detection: the user's game list (by exe name) plus a fullscreen
//! heuristic for the foreground window. Polled every couple of seconds by the
//! app; one call costs a process snapshot and a few window queries (< 1 ms).

use serde::Serialize;

use crate::capture::windows_list::{self, ProcessEntry, WindowInfo};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownGame {
    pub exe: String,
    pub name: String,
    pub profile: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedGame {
    pub exe: String,
    pub name: String,
    pub pid: u32,
    pub profile_id: Option<String>,
    pub fullscreen: bool,
    #[serde(skip)]
    pub hwnd: Option<isize>,
}

/// Fullscreen apps that are not games.
const NOT_GAMES: &[&str] = &[
    "explorer.exe", "chrome.exe", "msedge.exe", "firefox.exe", "opera.exe", "brave.exe", "vlc.exe",
    "mpc-hc64.exe", "mpc-be64.exe", "mpv.exe", "potplayermini64.exe", "wmplayer.exe", "Video.UI.exe",
    "ApplicationFrameHost.exe", "powerpnt.exe", "obs64.exe", "Code.exe", "devenv.exe", "WindowsTerminal.exe",
    "lanes.exe", "msedgewebview2.exe", "ShellExperienceHost.exe", "StartMenuExperienceHost.exe",
    "SearchHost.exe", "LockApp.exe", "Discord.exe", "Spotify.exe", "Teams.exe", "ms-teams.exe", "slack.exe",
];

/// Finds the game to capture: the foreground window if it is a known game or
/// (with the heuristic) a fullscreen non-blacklisted app; otherwise any running known game.
pub fn detect(known: &[KnownGame], fullscreen_heuristic: bool) -> Option<DetectedGame> {
    let fg = windows_list::foreground();
    let procs = windows_list::processes();
    // Only enumerate all windows when the cheap checks found nothing.
    pick(known, fullscreen_heuristic, fg.as_ref(), &procs).or_else(|| {
        fullscreen_heuristic.then(|| background_fullscreen(&windows_list::list())).flatten()
    })
}

fn is_candidate(w: &WindowInfo) -> bool {
    w.fullscreen && !w.exe.is_empty() && !NOT_GAMES.iter().any(|n| n.eq_ignore_ascii_case(&w.exe))
}

/// A borderless/fullscreen window that is not focused (e.g. a game on another
/// monitor while you use a second screen).
fn background_fullscreen(windows: &[WindowInfo]) -> Option<DetectedGame> {
    windows.iter().find(|w| is_candidate(w)).map(|w| DetectedGame {
        exe: w.exe.clone(),
        name: if w.title.is_empty() { w.exe.trim_end_matches(".exe").to_string() } else { w.title.clone() },
        pid: w.pid,
        profile_id: None,
        fullscreen: true,
        hwnd: Some(w.hwnd),
    })
}

/// True while the process of a detected game is still running.
pub fn still_running(game: &DetectedGame) -> bool {
    windows_list::processes().iter().any(|p| p.pid == game.pid && p.exe.eq_ignore_ascii_case(&game.exe))
}

fn pick(known: &[KnownGame], heuristic: bool, fg: Option<&WindowInfo>, procs: &[ProcessEntry]) -> Option<DetectedGame> {
    let lookup = |exe: &str| known.iter().find(|g| g.exe.eq_ignore_ascii_case(exe));
    if let Some(w) = fg {
        if let Some(g) = lookup(&w.exe) {
            return Some(DetectedGame {
                exe: w.exe.clone(),
                name: g.name.clone(),
                pid: w.pid,
                profile_id: g.profile.clone(),
                fullscreen: w.fullscreen,
                hwnd: Some(w.hwnd),
            });
        }
        if heuristic && is_candidate(w) {
            return Some(DetectedGame {
                exe: w.exe.clone(),
                name: if w.title.is_empty() { w.exe.trim_end_matches(".exe").to_string() } else { w.title.clone() },
                pid: w.pid,
                profile_id: None,
                fullscreen: true,
                hwnd: Some(w.hwnd),
            });
        }
    }
    // A known game running in the background (alt-tabbed).
    procs.iter().find_map(|p| {
        lookup(&p.exe).map(|g| DetectedGame {
            exe: p.exe.clone(),
            name: g.name.clone(),
            pid: p.pid,
            profile_id: g.profile.clone(),
            fullscreen: false,
            hwnd: None,
        })
    })
}

/// Resolves the window to capture for a detected game.
pub fn game_window(game: &DetectedGame) -> Option<WindowInfo> {
    match game.hwnd {
        Some(h) => windows_list::info(windows::Win32::Foundation::HWND(h as _)),
        None => windows_list::main_window_of(&game.exe),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(exe: &str, fullscreen: bool) -> WindowInfo {
        WindowInfo { hwnd: 1, title: "T".into(), exe: exe.into(), pid: 42, fullscreen, rect: (0, 0, 1920, 1080) }
    }

    fn known() -> Vec<KnownGame> {
        vec![KnownGame { exe: "cs2.exe".into(), name: "Counter-Strike 2".into(), profile: Some("fps".into()) }]
    }

    #[test]
    fn known_foreground_game_wins() {
        let g = pick(&known(), false, Some(&win("CS2.EXE", false)), &[]).unwrap();
        assert_eq!(g.name, "Counter-Strike 2");
        assert_eq!(g.profile_id.as_deref(), Some("fps"));
        assert_eq!(g.hwnd, Some(1));
    }

    #[test]
    fn fullscreen_heuristic_skips_browsers_and_respects_toggle() {
        assert!(pick(&known(), true, Some(&win("chrome.exe", true)), &[]).is_none());
        assert!(pick(&known(), true, Some(&win("eldenring.exe", true)), &[]).is_some());
        assert!(pick(&known(), false, Some(&win("eldenring.exe", true)), &[]).is_none());
        assert!(pick(&known(), true, Some(&win("eldenring.exe", false)), &[]).is_none());
    }

    #[test]
    fn unfocused_fullscreen_window_is_found() {
        let windows = vec![win("Code.exe", false), win("firefox.exe", true), win("BNSR.exe", true)];
        assert_eq!(background_fullscreen(&windows).unwrap().exe, "BNSR.exe");
        assert!(background_fullscreen(&windows[..2]).is_none());
    }

    #[test]
    fn background_known_game_is_found_in_process_list() {
        let procs = vec![ProcessEntry { pid: 7, parent_pid: 1, exe: "cs2.exe".into() }];
        let g = pick(&known(), true, Some(&win("explorer.exe", false)), &procs).unwrap();
        assert_eq!(g.pid, 7);
        assert_eq!(g.hwnd, None);
    }
}
