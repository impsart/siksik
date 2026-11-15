mod compress;
mod jpeg;
mod settings;
mod watcher;

use compress::{FileResult, Job};
use rayon::prelude::*;
use serde::Serialize;
use settings::{Settings, Stats};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};
use watcher::WatchManager;

#[derive(Default, Clone, Serialize)]
struct Session {
    optimized: u64,
    skipped: u64,
    failed: u64,
    bytes_before: u64,
    bytes_after: u64,
}

struct AppState {
    settings: Arc<Mutex<Settings>>,
    stats: Arc<Mutex<Stats>>,
    session: Arc<Mutex<Session>>,
    watch: Mutex<Option<WatchManager>>,
}

#[derive(Serialize, Clone)]
struct StatsPayload {
    lifetime: Stats,
    session: Session,
}

#[derive(Serialize, Clone)]
struct BatchEvent {
    id: u64,
    total: usize,
    source: String,
}

#[derive(Serialize, Clone)]
struct WatchStatus {
    active: Vec<String>,
    errors: Vec<String>,
}

static BATCH: AtomicU64 = AtomicU64::new(1);

fn record(app: &AppHandle, r: &FileResult) {
    let st = app.state::<AppState>();
    let mut s = st.session.lock().unwrap();
    let mut l = st.stats.lock().unwrap();
    match r.status.as_str() {
        "optimized" => {
            s.optimized += 1;
            l.files_optimized += 1;
            s.bytes_before += r.before;
            s.bytes_after += r.after;
            l.bytes_before += r.before;
            l.bytes_after += r.after;
        }
        "skipped" => {
            s.skipped += 1;
            l.files_skipped += 1;
        }
        _ => {
            s.failed += 1;
            l.files_failed += 1;
        }
    }
    l.save();
}

fn run_jobs(app: AppHandle, jobs: Vec<Job>, source: &'static str) -> usize {
    let n = jobs.len();
    if n == 0 {
        return 0;
    }
    let id = BATCH.fetch_add(1, Ordering::SeqCst);
    let settings = app.state::<AppState>().settings.lock().unwrap().clone();
    let _ = app.emit("batch-start", BatchEvent { id, total: n, source: source.into() });
    std::thread::spawn(move || {
        jobs.par_iter().for_each(|job| {
            let r = compress::process(job, &settings, source);
            record(&app, &r);
            let _ = app.emit("file-done", &r);
        });
        let st = app.state::<AppState>();
        let _ = app.emit("batch-end", BatchEvent { id, total: n, source: source.into() });
        let _ = app.emit("stats", stats_payload(&st));
    });
    n
}

fn stats_payload(st: &AppState) -> StatsPayload {
    StatsPayload {
        lifetime: st.stats.lock().unwrap().clone(),
        session: st.session.lock().unwrap().clone(),
    }
}

fn apply_icon(app: &AppHandle, s: &Settings) {
    let Some(win) = app.get_webview_window("main") else { return };
    let img = if s.custom_icon.is_empty() {
        app.default_window_icon().cloned()
    } else {
        tauri::image::Image::from_path(&s.custom_icon).ok()
    };
    if let Some(img) = img {
        let _ = win.set_icon(img);
    }
}

// ---------- commands ----------

#[tauri::command]
fn get_settings(st: State<AppState>) -> Settings {
    st.settings.lock().unwrap().clone()
}

#[tauri::command]
fn save_settings(app: AppHandle, st: State<AppState>, settings: Settings) -> Result<WatchStatus, String> {
    settings.save().map_err(|e| e.to_string())?;
    *st.settings.lock().unwrap() = settings.clone();
    apply_icon(&app, &settings);
    let mut w = st.watch.lock().unwrap();
    let w = w.as_mut().unwrap();
    w.apply(&settings);
    Ok(WatchStatus { active: w.active.clone(), errors: w.errors.clone() })
}

#[tauri::command]
fn watch_status(st: State<AppState>) -> WatchStatus {
    let w = st.watch.lock().unwrap();
    let w = w.as_ref().unwrap();
    WatchStatus { active: w.active.clone(), errors: w.errors.clone() }
}

#[tauri::command]
fn get_stats(st: State<AppState>) -> StatsPayload {
    stats_payload(&st)
}

#[tauri::command]
fn reset_stats(st: State<AppState>, lifetime: bool) -> StatsPayload {
    *st.session.lock().unwrap() = Session::default();
    if lifetime {
        let mut l = st.stats.lock().unwrap();
        *l = Stats { first_used: settings::now(), ..Default::default() };
        l.save();
    }
    stats_payload(&st)
}

#[tauri::command]
fn compress_paths(app: AppHandle, st: State<AppState>, paths: Vec<String>) -> usize {
    let s = st.settings.lock().unwrap().clone();
    let jobs = compress::collect(&paths, &s);
    run_jobs(app, jobs, "manual")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let settings = Arc::new(Mutex::new(Settings::load()));
    let state = AppState {
        settings: settings.clone(),
        stats: Arc::new(Mutex::new(Stats::load())),
        session: Default::default(),
        watch: Mutex::new(None),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .setup(move |app| {
            let handle = app.handle().clone();
            let h2 = handle.clone();
            let handler: watcher::Handler = Arc::new(move |jobs| {
                run_jobs(h2.clone(), jobs, "watch");
            });
            let mut wm = WatchManager::new(settings.clone(), handler);
            let s = settings.lock().unwrap().clone();
            wm.apply(&s);
            *app.state::<AppState>().watch.lock().unwrap() = Some(wm);
            apply_icon(&handle, &s);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            save_settings,
            watch_status,
            get_stats,
            reset_stats,
            compress_paths
        ])
        .run(tauri::generate_context!())
        .expect("error while running Siksik");
}
