use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use gpui::{App, AppContext, AsyncApp};

use crate::protocol::{RuntimeEvent, RuntimeEventKind};
use crate::PlatformStateClient;

pub fn spawn_runtime_event_router<F>(cx: &mut App, kinds: Vec<RuntimeEventKind>, mut on_event: F)
where
    F: FnMut(&mut App, &RuntimeEvent) + 'static,
{
    let (tx, rx) = mpsc::channel::<RuntimeEvent>();
    spawn_subscriber(kinds, tx);
    let rx = Arc::new(Mutex::new(rx));
    cx.spawn(async move |cx: &mut AsyncApp| loop {
        let received = {
            let rx = rx.clone();
            cx.background_spawn(async move { rx.lock().ok()?.recv().ok() })
                .await
        };
        let Some(first) = received else {
            break;
        };
        let latest = coalesce(&rx, first);
        let _ = cx.update(|app| on_event(app, &latest));
    })
    .detach();
}

fn spawn_subscriber(kinds: Vec<RuntimeEventKind>, tx: mpsc::Sender<RuntimeEvent>) {
    std::thread::spawn(move || {
        let client = PlatformStateClient::from_env();
        let Some(mut subscription) = client.subscribe(kinds) else {
            return;
        };
        while let Some(event) = subscription.next_event() {
            if tx.send(event).is_err() {
                return;
            }
        }
    });
}

fn coalesce(
    rx: &Arc<Mutex<mpsc::Receiver<RuntimeEvent>>>,
    mut latest: RuntimeEvent,
) -> RuntimeEvent {
    if let Ok(guard) = rx.lock() {
        while let Ok(event) = guard.try_recv() {
            latest = event;
        }
    }
    latest
}
