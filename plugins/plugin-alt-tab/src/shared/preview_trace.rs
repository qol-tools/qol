use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

#[derive(Clone, Copy)]
struct Stamp {
    source: &'static str,
    at: Instant,
}

pub(crate) struct Snapshot {
    pub source: &'static str,
    pub age_ms: u128,
}

fn shared_stamps() -> &'static Mutex<HashMap<u32, Stamp>> {
    static STAMPS: OnceLock<Mutex<HashMap<u32, Stamp>>> = OnceLock::new();
    STAMPS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn live_stamps() -> &'static Mutex<HashMap<u32, Stamp>> {
    static STAMPS: OnceLock<Mutex<HashMap<u32, Stamp>>> = OnceLock::new();
    STAMPS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn record_shared_fill(ids: impl IntoIterator<Item = u32>) {
    record_many(shared_stamps(), ids, "fill");
}

pub(crate) fn record_live_update(wid: u32) {
    record_many(live_stamps(), [wid], "live");
}

pub(crate) fn shared_snapshot(wid: u32) -> Option<Snapshot> {
    snapshot(shared_stamps(), wid)
}

pub(crate) fn live_snapshot(wid: u32) -> Option<Snapshot> {
    snapshot(live_stamps(), wid)
}

pub(crate) fn retain_active(active: &HashSet<u32>) {
    if let Ok(mut stamps) = shared_stamps().lock() {
        stamps.retain(|wid, _| active.contains(wid));
    }
    if let Ok(mut stamps) = live_stamps().lock() {
        stamps.retain(|wid, _| active.contains(wid));
    }
}

fn record_many(
    stamps: &'static Mutex<HashMap<u32, Stamp>>,
    ids: impl IntoIterator<Item = u32>,
    source: &'static str,
) {
    let now = Instant::now();
    if let Ok(mut stamps) = stamps.lock() {
        for id in ids {
            stamps.insert(id, Stamp { source, at: now });
        }
    }
}

fn snapshot(stamps: &'static Mutex<HashMap<u32, Stamp>>, wid: u32) -> Option<Snapshot> {
    stamps
        .lock()
        .ok()?
        .get(&wid)
        .copied()
        .map(|stamp| Snapshot {
            source: stamp.source,
            age_ms: stamp.at.elapsed().as_millis(),
        })
}
