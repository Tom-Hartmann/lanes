//! System tray icon and menu. The tray is the app's permanent presence; the
//! UI window comes and goes. The menu reflects the engine state and is only
//! rebuilt when that state changes.

use std::sync::Mutex;

use lanes_core::Timestamp;
use lanes_core::engine::{EngineStatus, RecordingState};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::actions::{self, Action, Trigger};
use crate::state::AppState;
use crate::window;

const TRAY_ID: &str = "main";

/// (recording, paused, replay) the menu was last built for.
static SHOWN: Mutex<Option<(bool, bool, bool)>> = Mutex::new(None);

fn flags(status: Option<&EngineStatus>) -> (bool, bool, bool) {
    match status {
        Some(s) => {
            let (rec, paused) = match &s.recording {
                RecordingState::Idle => (false, false),
                RecordingState::Recording { paused, .. } => (true, *paused),
            };
            (rec, paused, s.replay.active)
        }
        None => (false, false, false),
    }
}

fn build_menu(app: &AppHandle, (recording, paused, replay): (bool, bool, bool)) -> tauri::Result<Menu<Wry>> {
    let state = app.state::<AppState>();
    let settings = state.settings();
    let none = None::<&str>;
    let rec = MenuItem::with_id(app, "toggle_recording", if recording { "Stop recording" } else { "Start recording" }, true, none)?;
    let pause = MenuItem::with_id(app, "toggle_pause", if paused { "Resume recording" } else { "Pause recording" }, recording, none)?;
    let replay_item =
        MenuItem::with_id(app, "toggle_replay", if replay { "Stop replay buffer" } else { "Start replay buffer" }, true, none)?;
    let clips = Submenu::with_id(app, "clips", "Save clip", replay)?;
    for p in &settings.clip_presets {
        clips.append(&MenuItem::with_id(app, Action::SaveClip(p.id.clone()).id(), &p.name, replay, none)?)?;
    }
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "open", "Open Lanes", true, none)?,
            &PredefinedMenuItem::separator(app)?,
            &rec,
            &pause,
            &replay_item,
            &clips,
            &PredefinedMenuItem::separator(app)?,
        ],
    )?;
    if !state.elevated && lanes_platform::admin::task_exists() {
        menu.append(&MenuItem::with_id(app, "relaunch_admin", "Restart as administrator", true, none)?)?;
    }
    menu.append(&MenuItem::with_id(app, "quit", "Quit Lanes", true, none)?)?;
    Ok(menu)
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app, (false, false, false))?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Lanes")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => window::show_main(app),
            "quit" => {
                log::info!("quitting");
                app.exit(0);
            }
            "relaunch_admin" => crate::commands::relaunch_elevated(app),
            id => {
                if let Some(action) = Action::parse(id) {
                    actions::dispatch(app, &action, Trigger::Tray, Timestamp::now(), None);
                }
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                window::show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Rebuilds the menu if recording/replay state changed, and updates the tooltip.
pub fn update_status(app: &AppHandle, status: &EngineStatus) {
    let f = flags(Some(status));
    {
        let mut shown = SHOWN.lock().unwrap_or_else(|e| e.into_inner());
        if *shown == Some(f) {
            return;
        }
        *shown = Some(f);
    }
    let tooltip = match f {
        (true, true, _) => "Lanes · recording paused",
        (true, false, true) => "Lanes · recording + replay buffer",
        (true, false, false) => "Lanes · recording",
        (false, _, true) => "Lanes · replay buffer on",
        _ => "Lanes",
    };
    let app2 = app.clone();
    // Menu changes must happen on the main thread.
    let _ = app.run_on_main_thread(move || {
        if let Some(tray) = app2.tray_by_id(TRAY_ID) {
            if let Ok(menu) = build_menu(&app2, f) {
                let _ = tray.set_menu(Some(menu));
            }
            let _ = tray.set_tooltip(Some(tooltip));
        }
    });
}

/// Rebuilds the menu (e.g. clip presets changed).
pub fn refresh(app: &AppHandle) {
    *SHOWN.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let status = app.state::<AppState>().status();
    if let Some(s) = status {
        update_status(app, &s);
    }
}
