//! GPUI-side daemon command loop.
//!
//! Pairs with [`crate::daemon::start_listener`], which parses socket traffic
//! into a `Cmd` and pushes it onto an `mpsc::Sender`. This loop owns the
//! receiver end: it pulls each `Cmd` off a background thread, then runs the
//! caller's `handler` on the app executor.
//!
//! The app quits when the handler returns [`LoopFlow::Stop`] or the channel
//! disconnects. A popup daemon with no command source cannot do anything
//! useful, so both exits are terminal; callers do not need their own quit
//! call in the handler.

use std::future::Future;
use std::sync::mpsc::Receiver;

use futures::{channel::mpsc, StreamExt};
use gpui::{App, AsyncApp};

/// Whether the command loop keeps waiting or stops (and quits the app).
pub enum LoopFlow {
    Continue,
    Stop,
}

pub fn spawn_command_loop<Cmd, H, F>(cx: &mut App, rx: Receiver<Cmd>, mut handler: H)
where
    Cmd: Send + 'static,
    H: FnMut(AsyncApp, Cmd) -> F + 'static,
    F: Future<Output = LoopFlow> + 'static,
{
    let (tx, mut async_rx) = mpsc::unbounded();
    let _ = std::thread::Builder::new()
        .name("qol-gpui-command-loop".into())
        .spawn(move || {
            while let Ok(command) = rx.recv() {
                if tx.unbounded_send(command).is_err() {
                    return;
                }
            }
        });
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Some(cmd) = async_rx.next().await {
            if matches!(handler(cx.clone(), cmd).await, LoopFlow::Stop) {
                break;
            }
        }
        let _ = cx.update(|app| app.quit());
    })
    .detach();
}
