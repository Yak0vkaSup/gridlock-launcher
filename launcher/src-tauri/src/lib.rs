//! GridLock launcher: sign in through the site, install the newest build, keep it updated, play.

mod auth;
mod config;
mod delta;
mod host;
mod install;
mod net;

use net::{ApiError, Manifest, UserInfo};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, State};

#[cfg(windows)]
pub const PLATFORM: &str = "windows";
#[cfg(not(windows))]
pub const PLATFORM: &str = "linux";

#[derive(Default)]
pub struct AppState {
    manifest: Mutex<Option<Manifest>>,
}

#[derive(Serialize)]
pub struct StateInfo {
    platform: &'static str,
    launcher_version: &'static str,
    site: String,
    logged_in: bool,
    install_dir: String,
    installed_version: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct CheckInfo {
    latest: String,
    installed: Option<String>,
    files: usize,
    bytes: u64,
    up_to_date: bool,
}

fn api_err(e: ApiError) -> String {
    if matches!(e, ApiError::Unauthorized) {
        // the token is dead: forget it so the UI falls back to Sign in
        let mut cfg = config::load();
        cfg.token = None;
        let _ = config::save(&cfg);
    }
    e.to_string()
}

fn token() -> Result<String, String> {
    config::load().token.ok_or_else(|| "unauthorized".to_string())
}

#[tauri::command]
fn get_state() -> StateInfo {
    let cfg = config::load();
    let dir = cfg.install_dir();
    StateInfo {
        platform: PLATFORM,
        launcher_version: env!("CARGO_PKG_VERSION"),
        site: cfg.site(),
        logged_in: cfg.token.is_some(),
        install_dir: dir.display().to_string(),
        installed_version: install::load_installed(&dir).map(|i| i.version),
    }
}

#[tauri::command]
async fn login(app: AppHandle) -> Result<UserInfo, String> {
    let mut cfg = config::load();
    let site = cfg.site();
    let tok = auth::login(&app, &site).await.map_err(|e| e.to_string())?;
    let user = net::me(&site, &tok).await.map_err(|e| e.to_string())?;
    cfg.token = Some(tok);
    config::save(&cfg).map_err(|e| e.to_string())?;
    Ok(user)
}

#[tauri::command]
fn logout() -> Result<(), String> {
    let mut cfg = config::load();
    cfg.token = None;
    config::save(&cfg).map_err(|e| e.to_string())
}

#[tauri::command]
async fn whoami() -> Result<UserInfo, String> {
    let cfg = config::load();
    net::me(&cfg.site(), &token()?).await.map_err(api_err)
}

fn check_with(manifest: &Manifest, dir: &std::path::Path) -> Result<CheckInfo, String> {
    let installed = install::load_installed(dir);
    let plan = install::plan(dir, manifest, installed.as_ref()).map_err(|e| e.to_string())?;
    Ok(CheckInfo {
        latest: manifest.version.clone(),
        installed: installed.map(|i| i.version),
        files: plan.to_download.len(),
        bytes: plan.bytes,
        up_to_date: plan.to_download.is_empty() && plan.to_delete.is_empty(),
    })
}

#[tauri::command]
async fn check(state: State<'_, AppState>) -> Result<CheckInfo, String> {
    let cfg = config::load();
    let manifest = net::manifest(&cfg.site(), &token()?, PLATFORM).await.map_err(api_err)?;
    let info = check_with(&manifest, &cfg.install_dir())?;
    *state.manifest.lock().unwrap() = Some(manifest);
    Ok(info)
}

#[tauri::command]
async fn install(app: AppHandle, state: State<'_, AppState>) -> Result<CheckInfo, String> {
    let cfg = config::load();
    let dir = cfg.install_dir();
    // presigned urls live for an hour; fetch a fresh manifest if the cached one is stale or missing
    let cached = state.manifest.lock().unwrap().clone();
    let fresh_enough = cached.as_ref().and_then(|m| m.url_expires_at).is_some_and(|t| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        t > now + 5 * 60 * 1000
    });
    let manifest = match cached {
        Some(m) if fresh_enough => m,
        _ => net::manifest(&cfg.site(), &token()?, PLATFORM).await.map_err(api_err)?,
    };
    let installed = install::load_installed(&dir);
    let plan = install::plan(&dir, &manifest, installed.as_ref()).map_err(|e| e.to_string())?;
    install::apply(app, dir.clone(), &manifest, plan).await.map_err(|e| e.to_string())?;
    let info = check_with(&manifest, &dir)?;
    *state.manifest.lock().unwrap() = Some(manifest);
    Ok(info)
}

#[tauri::command]
fn verify(app: AppHandle) -> Result<usize, String> {
    let cfg = config::load();
    install::verify(&cfg.install_dir(), &app).map_err(|e| e.to_string())
}

#[tauri::command]
fn play() -> Result<(), String> {
    let cfg = config::load();
    let dir = cfg.install_dir();
    let installed = install::load_installed(&dir).ok_or("the game is not installed")?;
    let exe = if installed.exec.is_empty() {
        if cfg!(windows) { "GridLock.exe" } else { "GridLock.sh" }.to_string()
    } else {
        installed.exec
    };
    let path = dir.join(&exe);
    if !path.exists() {
        return Err(format!("{} is missing; run Verify", path.display()));
    }
    // the host's environment, not the AppImage's (host.rs): the game must find its own libraries
    host::command(&path)
        .current_dir(&dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not start {}: {e}", path.display()))
}

#[tauri::command]
fn set_install_dir(path: String) -> Result<String, String> {
    let mut cfg = config::load();
    let p = std::path::PathBuf::from(path.trim());
    if p.as_os_str().is_empty() {
        return Err("empty path".into());
    }
    // keep the game in its own folder so an update never deletes anything else
    let p = if p.file_name().map(|n| n == "GridLock").unwrap_or(false) { p } else { p.join("GridLock") };
    std::fs::create_dir_all(&p).map_err(|e| e.to_string())?;
    cfg.install_dir = Some(p.clone());
    config::save(&cfg).map_err(|e| e.to_string())?;
    Ok(p.display().to_string())
}

#[tauri::command]
fn open_install_dir() -> Result<(), String> {
    let dir = config::load().install_dir();
    host::open_path(&dir).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            get_state,
            login,
            logout,
            whoami,
            check,
            install,
            verify,
            play,
            set_install_dir,
            open_install_dir
        ])
        .run(tauri::generate_context!())
        .expect("error while running the GridLock launcher");
}
