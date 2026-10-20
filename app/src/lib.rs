//! Lanes desktop shell: bridges `lanes-core` to the React UI, and owns the
//! tray icon, global hotkeys, settings, autostart and admin mode.

mod actions;
mod bindings;
mod commands;
mod detect;
mod feedback;
mod hotkeys;
mod instance;
mod library;
mod logbus;
mod settings;
mod state;
mod tray;
mod window;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use lanes_core::engine::Engine;
use lanes_platform::{admin, autostart};
use tauri::{Manager, RunEvent};

/// Written by a standard instance before it restarts elevated, so the
/// elevated instance opens its window instead of starting minimized.
pub(crate) fn show_marker() -> PathBuf {
    state::local_data_dir().join("show-on-start")
}

/// How Lanes was launched.
struct Launch {
    /// Started by Windows at logon (Run key or the elevated task's trigger).
    autostart: bool,
    from_task: bool,
    show_requested: bool,
    minimized_flag: bool,
}

impl Launch {
    fn from_args(args: &[String]) -> Self {
        let has = |a: &str| args.iter().any(|x| x == a);
        let show_requested = std::fs::remove_file(show_marker()).is_ok();
        let from_task = has(admin::FROM_TASK_ARG);
        Self {
            autostart: has(autostart::AUTOSTART_ARG) || (from_task && !show_requested),
            from_task,
            show_requested,
            minimized_flag: has("--minimized"),
        }
    }
}

pub fn run() {
    logbus::install();
    let args: Vec<String> = std::env::args().collect();

    // Elevated helper mode (`--admin-task install|remove`): do the job and exit
    // without touching the single-instance lock or opening any UI.
    if let Some(code) = admin::handle_cli(&args) {
        std::process::exit(code);
    }
    let launch = Launch::from_args(&args);

    // An elevated instance started by "Restart as administrator" may race the
    // exiting standard instance for the lock: retry briefly.
    let deadline = Instant::now() + if launch.from_task { Duration::from_secs(10) } else { Duration::ZERO };
    let lock = loop {
        match instance::acquire() {
            Ok(instance::Acquire::Primary(lock)) => break Some(lock),
            Ok(instance::Acquire::AlreadyRunning) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(200)),
            Ok(instance::Acquire::AlreadyRunning) => {
                if !launch.autostart {
                    instance::signal_existing();
                }
                return;
            }
            Err(e) => {
                // Better to risk a second instance than to refuse to start.
                log::error!("single-instance lock failed: {e}");
                break None;
            }
        }
    };
    let elevated = instance::is_elevated();

    let store = settings::Store::new(&settings::config_dir());
    let mut initial = store.load();
    if launch.autostart
        && let Some(p) = initial.general.autostart_profile.clone().filter(|p| initial.profile(p).is_some()) {
            initial.active_profile = p;
        }
    bindings::reconcile_settings(&mut initial);
    let show_window =
        launch.show_requested || !(launch.minimized_flag || (launch.autostart && initial.general.start_minimized));

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_app_info,
            commands::get_logs,
            commands::ui_log,
            commands::get_settings,
            commands::update_settings,
            commands::activate_profile,
            commands::list_hotkeys,
            commands::set_hotkey,
            commands::trigger_action,
            commands::get_status,
            commands::start_recording,
            commands::stop_recording,
            commands::toggle_pause,
            commands::start_replay,
            commands::stop_replay,
            commands::save_clip,
            commands::list_monitors,
            commands::get_monitor_thumbnail,
            commands::list_windows,
            commands::list_audio_devices,
            commands::list_audio_apps,
            commands::list_encoders,
            commands::list_clips,
            commands::get_clip_thumbnail,
            commands::prepare_clip_playback,
            commands::delete_clip,
            commands::reveal_clip,
            commands::open_output_folder,
            commands::remux_clip,
            commands::trim_clip,
            commands::set_run_as_admin,
            commands::relaunch_as_admin,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            logbus::attach(handle.clone());
            log::info!(
                "Lanes {} starting ({}{})",
                env!("CARGO_PKG_VERSION"),
                if elevated { "administrator" } else { "standard user" },
                if launch.autostart { ", autostart" } else { "" }
            );
            if let Some(lock) = lock {
                let show = handle.clone();
                lock.listen_for_show(move || window::show_main(&show))?;
                // Held until the process exits.
                app.manage(lock);
            }

            let engine = Engine::start(state::engine_config(&initial), state::make_sink(&handle))?;
            let hotkeys = bindings::Hotkeys::start(handle.clone())?;
            app.manage(state::AppState::new(initial.clone(), store, engine, hotkeys, elevated));
            let st = app.state::<state::AppState>();
            st.hotkeys.sync(&initial);
            state::allow_asset_folders(&handle, &st.library_folders());
            // Elevated: bring the task's logon trigger in line with the settings.
            if elevated || !initial.general.run_as_admin {
                state::reconcile_autostart(&initial, elevated);
            }

            tray::create(&handle)?;
            detect::start(handle.clone());

            if launch.autostart && initial.general.autostart_replay {
                let delay = Duration::from_secs(u64::from(initial.general.autostart_delay_secs));
                let h = handle.clone();
                let _ = std::thread::Builder::new().name("lanes-autostart".into()).spawn(move || {
                    // Let the desktop settle after logon before capturing.
                    std::thread::sleep(delay);
                    h.state::<state::AppState>().with_engine(|e| e.start_replay());
                });
            }
            if show_window {
                window::show_main(&handle);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to initialise Tauri");

    app.run(|app, event| match event {
        // Closing the last window must not quit: the backend lives on in the tray.
        // An explicit exit (tray "Quit") carries an exit code and goes through.
        RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
        RunEvent::WindowEvent { label, event: tauri::WindowEvent::Destroyed, .. } if label == window::MAIN => {
            log::info!("UI closed; Lanes keeps running in the tray");
        }
        RunEvent::Exit => {
            // Stop the engine so a running recording is finalised properly.
            if let Some(state) = app.try_state::<state::AppState>() {
                let engine = state.engine.lock().unwrap_or_else(|e| e.into_inner()).take();
                drop(engine);
            }
        }
        _ => {}
    });
}
