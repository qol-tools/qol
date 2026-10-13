mod app_name;
mod banner;
mod record;
mod signal;
mod store;
mod toast;

use std::io::{PipeReader, PipeWriter};
use std::path::Path;
use std::sync::Mutex;

use super::super::MirrorAccess;
use signal::{Signal, Wake};
use store::{Cursor, Session};

static MIRROR: Mutex<Option<PipeWriter>> = Mutex::new(None);

pub(in super::super) fn sync(enabled: bool) {
    let mut running = MIRROR
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    banner::sync(enabled);
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
        Ok(stop) => {
            log::info!("notification mirror: watching macOS notifications");
            *running = Some(stop);
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

fn start() -> anyhow::Result<PipeWriter> {
    let db = store::db_path().ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    let cursor = store::latest(&db)?;
    let (stopped, stop) = std::io::pipe()?;
    let signal = Signal::open(&db, &stopped)?;
    std::thread::Builder::new()
        .name("qol-notification-mirror".to_string())
        .spawn(move || run(&db, cursor, signal, stopped))?;
    Ok(stop)
}

fn run(db: &Path, mut cursor: Cursor, mut signal: Signal, _stopped: PipeReader) {
    let mut session = None;
    loop {
        match signal.wait() {
            Wake::Changed => {}
            Wake::Stop => return,
            Wake::Failed => {
                MIRROR
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take();
                return;
            }
        }
        if session.is_none() {
            session = Session::open(db)
                .inspect_err(|error| log::warn!("notification mirror: {error:#}"))
                .ok();
        }
        let Some(open) = session.as_mut() else {
            continue;
        };
        match open.since(cursor) {
            Ok(rows) => cursor = show_rows(rows, cursor),
            Err(error) => {
                log::warn!("notification mirror: read failed: {error:#}");
                session = None;
            }
        }
    }
}

fn show_rows(rows: Vec<store::Row>, mut cursor: Cursor) -> Cursor {
    for row in rows {
        cursor = row.cursor;
        if let Some(notification) = record::decode(&row.data) {
            if banner::claimed(&notification.title, &notification.body) {
                continue;
            }
            toast::show(&app_name::display_name(&notification.app_id), &notification);
        }
    }
    cursor
}
