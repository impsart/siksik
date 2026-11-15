//! Folder watching. New or moved-in images are queued, and processed once
//! their size has stopped changing (so half-copied files are never touched).

use crate::compress::{self, Job};
use crate::settings::Settings;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type Handler = Arc<dyn Fn(Vec<Job>) + Send + Sync>;

pub struct WatchManager {
    watcher: Option<RecommendedWatcher>,
    tx: Sender<PathBuf>,
    folders: Arc<Mutex<Vec<PathBuf>>>,
    pub active: Vec<String>,
    pub errors: Vec<String>,
}

impl WatchManager {
    pub fn new(settings: Arc<Mutex<Settings>>, handler: Handler) -> Self {
        let (tx, rx) = channel::<PathBuf>();
        let folders: Arc<Mutex<Vec<PathBuf>>> = Default::default();
        let folders2 = folders.clone();

        std::thread::spawn(move || {
            // path -> (last seen size, when it last changed)
            let mut pending: HashMap<PathBuf, (u64, Instant)> = HashMap::new();
            loop {
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(p) => {
                        pending.entry(p).or_insert((u64::MAX, Instant::now()));
                        continue; // drain bursts first
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                    Err(RecvTimeoutError::Timeout) => {}
                }
                let mut ready = vec![];
                pending.retain(|p, (size, changed)| {
                    let Ok(m) = std::fs::metadata(p) else { return false };
                    let len = m.len();
                    if len != *size {
                        *size = len;
                        *changed = Instant::now();
                        true
                    } else if changed.elapsed() >= Duration::from_millis(1500) && len > 0 {
                        ready.push(p.clone());
                        false
                    } else {
                        true
                    }
                });
                if ready.is_empty() {
                    continue;
                }
                let s = settings.lock().unwrap().clone();
                let roots = folders2.lock().unwrap().clone();
                let jobs: Vec<Job> = ready
                    .into_iter()
                    .filter(|p| !compress::recently_written(p) && !compress::is_own_copy(p, &s))
                    .map(|p| {
                        let base = roots.iter().find(|r| p.starts_with(r)).cloned();
                        Job { file: p, base }
                    })
                    .collect();
                if !jobs.is_empty() {
                    handler(jobs);
                }
            }
        });

        Self { watcher: None, tx, folders, active: vec![], errors: vec![] }
    }

    pub fn apply(&mut self, s: &Settings) {
        self.watcher = None;
        self.active.clear();
        self.errors.clear();
        self.folders.lock().unwrap().clear();
        if !s.watch_enabled || s.watch_folders.is_empty() {
            return;
        }
        let tx = self.tx.clone();
        let w = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(ev) = res else { return };
            if !matches!(ev.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                return;
            }
            if let EventKind::Modify(notify::event::ModifyKind::Metadata(_)) = ev.kind {
                return;
            }
            for p in ev.paths {
                if compress::is_supported(&p) && !compress::recently_written(&p) {
                    let _ = tx.send(p);
                }
            }
        });
        let mut w = match w {
            Ok(w) => w,
            Err(e) => {
                self.errors.push(e.to_string());
                return;
            }
        };
        let mode = if s.recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
        for f in &s.watch_folders {
            let p = PathBuf::from(f);
            match w.watch(&p, mode) {
                Ok(()) => {
                    self.active.push(f.clone());
                    self.folders.lock().unwrap().push(p);
                }
                Err(e) => self.errors.push(format!("{f}: {e}")),
            }
        }
        self.watcher = Some(w);
    }
}
