//! Game detection loop: every two seconds, look for a running game (user
//! list + fullscreen heuristic), tell the engine (follow-game capture) and
//! switch to the game's profile if configured. Costs well under 1 ms per poll.

use std::time::Duration;

use lanes_core::games::{self, DetectedGame, KnownGame};
use tauri::{AppHandle, Manager};

use crate::state::{self, AppState};

const POLL: Duration = Duration::from_secs(2);

pub fn start(app: AppHandle) {
    let spawned = std::thread::Builder::new().name("lanes-game-detect".into()).spawn(move || {
        let mut current: Option<DetectedGame> = None;
        loop {
            std::thread::sleep(POLL);
            let state = app.state::<AppState>();
            let settings = state.settings();
            let mut detected = if settings.game_detection.enabled {
                let known: Vec<KnownGame> = settings
                    .games
                    .iter()
                    .filter(|g| g.enabled)
                    .map(|g| KnownGame { exe: g.exe.clone(), name: g.name.clone(), profile: g.profile.clone() })
                    .collect();
                games::detect(&known, settings.game_detection.fullscreen_heuristic, &settings.game_detection.ignored)
            } else {
                None
            };
            // Alt-tabbing out (e.g. into Lanes) must not lose the game: keep it
            // while its process runs and nothing else was detected.
            if detected.is_none() && settings.game_detection.enabled
                && let Some(g) = current
                    .as_ref()
                    .filter(|g| !settings.game_detection.ignored.iter().any(|i| i.eq_ignore_ascii_case(&g.exe)))
                    .filter(|g| games::still_running(g)) {
                    detected = Some(g.clone());
                }
            let same = |a: &Option<DetectedGame>, b: &Option<DetectedGame>| {
                a.as_ref().map(|g| (&g.exe, g.pid, g.hwnd)) == b.as_ref().map(|g| (&g.exe, g.pid, g.hwnd))
            };
            if same(&detected, &current) {
                continue;
            }
            match &detected {
                Some(g) => log::info!("game detected: {} ({})", g.name, g.exe),
                None => log::info!("no game running"),
            }
            if settings.game_detection.auto_switch_profile {
                switch_profile(&app, &state, &settings, detected.as_ref());
            }
            state.with_engine(|e| e.set_game(detected.clone()));
            current = detected;
        }
    });
    if let Err(e) = spawned {
        log::error!("game detection unavailable: {e}");
    }
}

fn switch_profile(app: &AppHandle, state: &AppState, settings: &crate::settings::Settings, game: Option<&DetectedGame>) {
    let mut before = state.profile_before_game.lock().unwrap_or_else(|e| e.into_inner());
    let target = match game.and_then(|g| g.profile_id.clone()) {
        Some(p) if settings.profile(&p).is_some() => {
            if before.is_none() {
                *before = Some(settings.active_profile.clone());
            }
            p
        }
        _ => match before.take() {
            Some(p) => p,
            None => return,
        },
    };
    drop(before);
    if target != settings.active_profile {
        log::info!("switching to profile \"{target}\"");
        let mut s = settings.clone();
        s.active_profile = target;
        if let Err(e) = state::apply_settings(app, s) {
            log::error!("{e}");
        }
    }
}
