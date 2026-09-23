use futures::{channel::mpsc, StreamExt};
use gpui::{App, AsyncApp};

use crate::protocol::{RuntimeEvent, RuntimeEventKind};
use crate::PlatformStateClient;

pub fn spawn_runtime_event_router<F>(cx: &mut App, kinds: Vec<RuntimeEventKind>, mut on_event: F)
where
    F: FnMut(&mut App, &RuntimeEvent) + 'static,
{
    let (tx, mut rx) = mpsc::unbounded();
    spawn_subscriber(kinds, tx);
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Some(first) = rx.next().await {
            let latest = coalesce(&mut rx, first);
            let _ = cx.update(|app| on_event(app, &latest));
        }
    })
    .detach();
}

fn spawn_subscriber(kinds: Vec<RuntimeEventKind>, tx: mpsc::UnboundedSender<RuntimeEvent>) {
    std::thread::spawn(move || {
        let client = PlatformStateClient::from_env();
        let Some(mut subscription) = client.subscribe(kinds) else {
            return;
        };
        while let Some(event) = subscription.next_event() {
            if tx.unbounded_send(event).is_err() {
                return;
            }
        }
    });
}

fn coalesce(
    rx: &mut mpsc::UnboundedReceiver<RuntimeEvent>,
    mut latest: RuntimeEvent,
) -> RuntimeEvent {
    while let Ok(event) = rx.try_recv() {
        latest = event;
    }
    latest
}
