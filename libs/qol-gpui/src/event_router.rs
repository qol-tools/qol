//! Bridge runtime monitor/window events to an app-thread callback.
//!
//! Subscribes to the given [`RuntimeEventKind`]s on a background thread,
//! coalesces bursts, and runs `on_event` on the GPUI app thread once per
//! batch. A ghost popup uses this to stay positioned on the active monitor
//! while it is hidden, so the next show lands in the right place instantly.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use gpui::{App, AppContext, AsyncApp};

use crate::protocol::RuntimeEventKind;
use crate::PlatformStateClient;

pub fn spawn_runtime_event_router<F>(cx: &mut App, kinds: Vec<RuntimeEventKind>, mut on_event: F)
where
    F: FnMut(&mut App) + 'static,
{
    let (tx, rx) = mpsc::channel::<()>();
    spawn_subscriber(kinds, tx);
    let rx = Arc::new(Mutex::new(rx));
    cx.spawn(async move |cx: &mut AsyncApp| loop {
        let received = {
            let rx = rx.clone();
            cx.background_spawn(async move { rx.lock().ok()?.recv().ok() })
                .await
        };
        if received.is_none() {
            break;
        }
        coalesce(&rx);
        let _ = cx.update(|app| on_event(app));
    })
    .detach();
}

fn spawn_subscriber(kinds: Vec<RuntimeEventKind>, tx: mpsc::Sender<()>) {
    std::thread::spawn(move || {
        let client = PlatformStateClient::from_env();
        let Some(mut subscription) = client.subscribe(kinds) else {
            return;
        };
        while subscription.next_event().is_some() {
            if tx.send(()).is_err() {
                return;
            }
        }
    });
}

fn coalesce(rx: &Arc<Mutex<mpsc::Receiver<()>>>) {
    if let Ok(guard) = rx.lock() {
        while guard.try_recv().is_ok() {}
    }
}
