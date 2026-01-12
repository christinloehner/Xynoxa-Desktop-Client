pub mod api;
pub mod config;
pub mod db;
pub mod sync;

use keyring::Entry;
use std::path::PathBuf;
use std::sync::Mutex;
use sync::SyncHandle;
use tauri::State;

use crate::config::{AppConfig, ConfigManager};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Manager, WindowEvent};

const KEYRING_SERVICE_NEW: &str = "xynoxa-desktop-client";
const KEYRING_SERVICE_LEGACY: &str = "xynoxa-desktop-client";

struct AppState {
    sync_engine: Mutex<Option<SyncHandle>>, // Renamed type
    config_manager: Mutex<Option<ConfigManager>>,
}

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn login(state: State<AppState>, token: String) -> Result<String, String> {
    if !(token.starts_with("xyn-") || token.starts_with("syn-")) {
        return Err("Invalid token format. Token must start with 'xyn-'.".to_string());
    }

    // Save to Keyring (Best Effort)
    if let Ok(entry) = Entry::new(KEYRING_SERVICE_NEW, "auth-token") {
        let _ = entry.set_password(&token);
    }
    if let Ok(entry) = Entry::new(KEYRING_SERVICE_LEGACY, "auth-token") {
        let _ = entry.delete_credential();
    }

    // Save to Config (User Request)
    let raw = state.config_manager.lock().map_err(|_| "Lock fail")?;
    let cm = raw.as_ref().ok_or("Config not init")?;
    cm.update(None, None, Some(token), None)?;

    Ok("Login successful".to_string())
}

#[tauri::command]
fn logout(state: State<AppState>) -> Result<(), String> {
    // Clear Keyring
    if let Ok(entry) = Entry::new(KEYRING_SERVICE_NEW, "auth-token") {
        let _ = entry.delete_credential();
    }
    if let Ok(entry) = Entry::new(KEYRING_SERVICE_LEGACY, "auth-token") {
        let _ = entry.delete_credential();
    }

    // Clear Config
    let raw = state.config_manager.lock().map_err(|_| "Lock fail")?;
    let cm = raw.as_ref().ok_or("Config not init")?;
    // To clear, we can pass empty string or handle logic in update.
    // update takes Option<String>. If we pass explicit None it ignores.
    // Ideally update should take Option<Option<String>> for unset?
    // For now, let's just make sure we interpret empty string as none or just overwrite.
    // Actually, `update` logic: `if let Some(t) = token { config.auth_token = Some(t); }`.
    // It doesn't allow clearing. We'll fix `update` or just hack it with empty string for now if usage allows,
    // but better to manually lock and clear.

    let mut config = cm.config.lock().map_err(|_| "Lock fail")?;
    config.auth_token = None;
    drop(config);
    cm.save()?;

    Ok(())
}

#[tauri::command]
fn check_auth(state: State<AppState>) -> bool {
    // Check Config first
    if let Ok(raw) = state.config_manager.lock() {
        if let Some(cm) = raw.as_ref() {
            if let Ok(conf) = cm.config.lock() {
                if conf.auth_token.is_some() {
                    return true;
                }
            }
        }
    }

    // Fallback to Keyring
    if let Ok(entry) = Entry::new(KEYRING_SERVICE_NEW, "auth-token") {
        return entry.get_password().is_ok();
    }
    if let Ok(entry) = Entry::new(KEYRING_SERVICE_LEGACY, "auth-token") {
        return entry.get_password().is_ok();
    }
    false
}

#[tauri::command]
fn get_config(state: State<AppState>) -> Result<AppConfig, String> {
    let raw = state.config_manager.lock().map_err(|_| "Lock fail")?;
    let cm = raw.as_ref().ok_or("Config not init")?;
    let conf = cm.config.lock().map_err(|_| "Lock fail")?;
    Ok(conf.clone())
}

#[tauri::command]
fn save_config(
    state: State<AppState>,
    url: Option<String>,
    path: Option<String>,
    token: Option<String>,
    completed: Option<bool>,
) -> Result<(), String> {
    let raw = state.config_manager.lock().map_err(|_| "Lock fail")?;
    let cm = raw.as_ref().ok_or("Config not init")?;
    cm.update(url, path, token, completed)
}

#[tauri::command]
fn start_sync(state: State<AppState>, token: Option<String>) -> Result<String, String> {
    // Load config
    let raw = state.config_manager.lock().map_err(|_| "Lock fail")?;
    let cm = raw.as_ref().ok_or("Config not init")?;
    let conf = cm.config.lock().map_err(|_| "Lock fail")?;

    let path_str = conf.sync_path.clone().ok_or("No sync path configured")?;
    let config_token = conf.auth_token.clone();

    // Expand ~
    let path_str = if path_str.starts_with("~/") {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        path_str.replacen("~", &home, 1)
    } else {
        path_str
    };

    let api_url = conf.server_url.clone(); // Clone before drop? yes.

    drop(conf); // Unlock early
    drop(raw);

    // Load token
    let auth_token = if let Some(t) = token {
        t
    } else if let Some(t) = config_token {
        t
    } else {
        if let Ok(entry) = Entry::new(KEYRING_SERVICE_NEW, "auth-token") {
            entry
                .get_password()
                .map_err(|_| "Not logged in".to_string())?
        } else if let Ok(entry) = Entry::new(KEYRING_SERVICE_LEGACY, "auth-token") {
            entry
                .get_password()
                .map_err(|_| "Not logged in".to_string())?
        } else {
            return Err("Not logged in".to_string());
        }
    };

    // Init Handle
    let mut engine_guard = state
        .sync_engine
        .lock()
        .map_err(|_| "Failed to lock state".to_string())?;

    // Prevent parallel worker instances (prevents duplicate folder creates/uploads)
    if engine_guard.is_some() {
        log::info!("Sync already running - skipping second start");
        return Ok("Sync already running".to_string());
    }

    // Create Handle (which spawns Worker)
    let handle = SyncHandle::new(auth_token, PathBuf::from(path_str), api_url);

    *engine_guard = Some(handle);
    Ok("Sync started".to_string())
}

#[tauri::command]
fn get_file_list(state: State<AppState>) -> Result<Vec<crate::db::FileRecord>, String> {
    let engine_guard = state
        .sync_engine
        .lock()
        .map_err(|_| "Failed to lock state".to_string())?;

    if let Some(handle) = &*engine_guard {
        handle.list_files()
    } else {
        Ok(vec![])
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            sync_engine: Mutex::new(None),
            config_manager: Mutex::new(None),
        })
        .setup(|app| {
            // 1. Setup Logging
            use simplelog::*;
            use std::fs::File;

            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            let log_dir = PathBuf::from(&home).join(".local/share/xynoxa/logs");
            std::fs::create_dir_all(&log_dir).ok();
            let log_path = log_dir.join("xynoxa.log");

            let _ = CombinedLogger::init(vec![
                TermLogger::new(
                    LevelFilter::Info,
                    Config::default(),
                    TerminalMode::Mixed,
                    ColorChoice::Auto,
                ),
                WriteLogger::new(
                    LevelFilter::Debug,
                    Config::default(),
                    File::create(&log_path).unwrap(),
                ),
            ]);

            log::info!("Application started");

            // Panics to log
            std::panic::set_hook(Box::new(move |info| {
                log::error!("Panic: {:?}", info);
            }));

            let _handle = app.handle();
            let app_data_dir = match app.path().app_data_dir() {
                Ok(dir) => dir,
                Err(e) => {
                    log::error!("Failed to resolve app_data_dir: {}", e);
                    return Ok(());
                }
            };

            // 2. Init Config
            let cm = ConfigManager::new(&app_data_dir);
            let state = app.state::<AppState>();

            // Acquire lock to check config status
            let mut conf_guard = state.config_manager.lock().unwrap();
            *conf_guard = Some(cm);

            // We need to access the inner config to check setup_completed
            let setup_completed = if let Some(manager) = conf_guard.as_ref() {
                manager.config.lock().unwrap().setup_completed
            } else {
                false
            };
            drop(conf_guard); // Release lock
            let window = match app.get_webview_window("main") {
                Some(w) => w,
                None => {
                    log::error!("Main window not found. Skipping UI setup.");
                    return Ok(());
                }
            };

            if setup_completed {
                // Try Config Token First
                let mut token_found = None;

                // Scope for lock
                {
                    let raw = state.config_manager.lock().unwrap();
                    if let Some(cm) = raw.as_ref() {
                        let conf = cm.config.lock().unwrap();
                        token_found = conf.auth_token.clone();
                    }
                }

                // Fallback to Keyring
                if token_found.is_none() {
                    if let Ok(entry) = Entry::new(KEYRING_SERVICE_NEW, "auth-token") {
                        if let Ok(t) = entry.get_password() {
                            token_found = Some(t);
                        }
                    }
                }
                if token_found.is_none() {
                    if let Ok(entry) = Entry::new(KEYRING_SERVICE_LEGACY, "auth-token") {
                        if let Ok(t) = entry.get_password() {
                            token_found = Some(t);
                        }
                    }
                }

                if let Some(token) = token_found {
                    log::info!("Setup complete and auth valid. Starting minimized.");

                    // Clone handle for background thread
                    let app_handle = app.handle().clone();

                    std::thread::spawn(move || {
                        let state = app_handle.state::<AppState>();

                        // Helper logic repeated for now to ensure correctness in setup context
                        let raw = state.config_manager.lock().unwrap();
                        let cm = raw.as_ref().unwrap();
                        let conf = cm.config.lock().unwrap();
                        let path_str = conf.sync_path.clone().unwrap_or_default();
                        // Expand ~ if present
                        let path_str = if path_str.starts_with("~/") {
                            let home_env =
                                std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                            path_str.replacen("~", &home_env, 1)
                        } else {
                            path_str
                        };
                        let api_url = conf.server_url.clone();
                        drop(conf);
                        drop(raw);

                        // SyncHandle::new starts the thread and watcher internally
                        let handle = SyncHandle::new(token, PathBuf::from(path_str), api_url);
                        *state.sync_engine.lock().unwrap() = Some(handle);
                        log::info!("Sync engine auto-started in background.");
                    });
                } else {
                    log::warn!("Auth token missing despite setup_completed. Showing wizard.");
                    if let Err(e) = window.show() {
                        log::error!("Failed to show window: {}", e);
                    }
                    if let Err(e) = window.set_focus() {
                        log::error!("Failed to focus window: {}", e);
                    }
                }
            } else {
                log::info!("Setup not complete. Showing wizard.");
                if let Err(e) = window.show() {
                    log::error!("Failed to show window: {}", e);
                }
                if let Err(e) = window.set_focus() {
                    log::error!("Failed to focus window: {}", e);
                }
            }

            // Setup Tray (optional; never crash app if unavailable)
            let quit_i = match MenuItem::with_id(app, "quit", "Quit", true, None::<&str>) {
                Ok(item) => item,
                Err(e) => {
                    log::warn!("Tray menu item 'quit' unavailable: {}", e);
                    return Ok(());
                }
            };
            let show_i = match MenuItem::with_id(app, "show", "Show", true, None::<&str>) {
                Ok(item) => item,
                Err(e) => {
                    log::warn!("Tray menu item 'show' unavailable: {}", e);
                    return Ok(());
                }
            };
            let menu = match Menu::with_items(app, &[&show_i, &quit_i]) {
                Ok(menu) => menu,
                Err(e) => {
                    log::warn!("Tray menu unavailable: {}", e);
                    return Ok(());
                }
            };

            if let Some(icon) = app.default_window_icon().cloned() {
                if let Err(e) = TrayIconBuilder::new()
                    .icon(icon)
                    .menu(&menu)
                    .on_menu_event(move |app, event| match event.id().as_ref() {
                        "quit" => {
                            app.exit(0);
                        }
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                if let Err(e) = window.show() {
                                    log::error!("Failed to show window: {}", e);
                                }
                                if let Err(e) = window.set_focus() {
                                    log::error!("Failed to focus window: {}", e);
                                }
                            }
                        }
                        _ => {}
                    })
                    .build(app)
                {
                    log::warn!("Tray initialization failed: {}", e);
                }
            } else {
                log::warn!("Tray icon unavailable. Skipping tray initialization.");
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                log::info!("Window Close Requested. Intercepting...");

                // Prevent close first to satisfy any OS constraints
                api.prevent_close();

                // Try to hide
                match window.hide() {
                    Ok(_) => {
                        log::info!("Window hidden successfully.");
                        // On Linux Wayland, sometimes hide() alone isn't enough or is ignored visually
                        // if the window thinks it's being closed.
                        // Force minimize as well to ensure it leaves the workspace.
                        #[cfg(target_os = "linux")]
                        {
                            let _ = window.minimize();
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to hide window: {}", e);
                        // Fallback to minimize if hide fails
                        let _ = window.minimize();
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            login,
            logout,
            check_auth,
            start_sync,
            get_file_list,
            get_config,
            save_config
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
