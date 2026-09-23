use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use chrono::Utc;
use notify::{EventKind, RecursiveMode, Watcher};

use crate::model::{Provider, ScanOutput};
use crate::store::RETENTION_DAYS;

/// How often to rescan and refresh even without file events (countdowns, missed events).
const TICK: Duration = Duration::from_secs(30);
/// Coalesces the burst of writes while a response is saved.
const DEBOUNCE: Duration = Duration::from_millis(250);

/// Watches every provider's log location on a background thread.
///
/// `on_update` gets new usage after file changes, and an empty update every `TICK` so
/// time-based figures stay current. `initial` is true for the first full scan.
pub fn spawn(mut providers: Vec<Box<dyn Provider>>, on_update: impl Fn(ScanOutput, bool) + Send + 'static) {
    thread::spawn(move || {
        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            // Our own reads (Access) and SQLite's shared-memory file would otherwise
            // trigger rescans in a loop.
            let relevant = matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_))
                && event.paths.iter().any(|p| !p.to_string_lossy().ends_with("-shm"));
            if relevant {
                let _ = tx.send(());
            }
        })
        .ok();
        let mut watched = HashSet::new();

        let scan = |providers: &mut Vec<Box<dyn Provider>>| {
            let cutoff = Utc::now() - chrono::Duration::days(RETENTION_DAYS);
            let mut out = ScanOutput::default();
            for provider in providers.iter_mut() {
                provider.scan(cutoff, &mut out);
            }
            out
        };

        on_update(scan(&mut providers), true);
        watch_new_paths(&providers, watcher.as_mut(), &mut watched);

        loop {
            match rx.recv_timeout(TICK) {
                Ok(()) => {
                    thread::sleep(DEBOUNCE);
                    while rx.try_recv().is_ok() {}
                    let out = scan(&mut providers);
                    if !out.events.is_empty() || !out.limits.is_empty() {
                        on_update(out, false);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    // Picks up log folders created after launch (a tool used for the first time).
                    watch_new_paths(&providers, watcher.as_mut(), &mut watched);
                    on_update(scan(&mut providers), false);
                }
                Err(RecvTimeoutError::Disconnected) => {
                    thread::sleep(TICK);
                    on_update(scan(&mut providers), false);
                }
            }
        }
    });
}

fn watch_new_paths(providers: &[Box<dyn Provider>], watcher: Option<&mut notify::RecommendedWatcher>, watched: &mut HashSet<PathBuf>) {
    let Some(watcher) = watcher else { return };
    for path in providers.iter().flat_map(|p| p.watch_paths()) {
        if path.exists() && !watched.contains(&path) && watcher.watch(&path, RecursiveMode::Recursive).is_ok() {
            watched.insert(path);
        }
    }
}
