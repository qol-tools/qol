mod app_name;
mod record;
mod store;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use super::super::MirrorAccess;
use store::Cursor;

const SETTLE: Duration = Duration::from_millis(150);

static MIRROR: Mutex<Option<qol_watch::Watch>> = Mutex::new(None);

pub(in super::super) fn sync(enabled: bool) {
    let mut running = MIRROR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !enabled {
        if running.take().is_some() {
            log::info!("notification mirror: stopped");
        }
        return;
    }
    if running.is_some() {
        return;
    }
    match start() {
        Ok(watch) => {
            log::info!("notification mirror: watching macOS notifications");
            *running = Some(watch);
        }
        Err(error) => log::warn!("notification mirror: not started: {error:#}"),
    }
}

pub(in super::super) fn access() -> MirrorAccess {
    let Some(db) = store::db_path() else {
        return MirrorAccess::Unreadable("HOME is not set".to_string());
    };
    match store::latest(&db) {
        Ok(_) => MirrorAccess::Readable,
        Err(error) => MirrorAccess::Unreadable(format!("{error:#}")),
    }
}

fn start() -> anyhow::Result<qol_watch::Watch> {
    let db = store::db_path().ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    let cursor = store::latest(&db)?;
    let dir = db
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{} has no parent", db.display()))?
        .to_path_buf();
    let (watch, batches) = qol_watch::settled(&[qol_watch::WatchRoot::shallow(dir)], SETTLE)?;
    std::thread::Builder::new()
        .name("qol-notification-mirror".to_string())
        .spawn(move || run(&db, cursor, &batches))?;
    Ok(watch)
}

fn run(db: &Path, mut cursor: Cursor, batches: &std::sync::mpsc::Receiver<Vec<PathBuf>>) {
    while batches.recv().is_ok() {
        let rows = match store::since(db, cursor) {
            Ok(rows) => rows,
            Err(error) => {
                log::warn!("notification mirror: read failed: {error:#}");
                continue;
            }
        };
        for row in rows {
            cursor = row.cursor;
            if let Some(notification) = record::decode(&row.data) {
                super::super::show(&app_name::display_name(&notification.app_id), &notification);
            }
        }
    }
}
