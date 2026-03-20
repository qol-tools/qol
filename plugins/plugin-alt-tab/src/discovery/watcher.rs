use super::CacheEvent;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ChangeWindowAttributesAux, ConnectionExt as _, EventMask};
use x11rb::protocol::Event;

const POLL_SLEEP: Duration = Duration::from_millis(50);

const WATCHED_ATOMS: &[&str] = &[
    "_NET_CLIENT_LIST_STACKING",
    "_NET_ACTIVE_WINDOW",
    "_NET_CLIENT_LIST",
];

pub struct WatcherHandle {
    shutdown_tx: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl WatcherHandle {
    pub fn stop(&mut self) {
        drop(self.shutdown_tx.take());
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn spawn_watcher(on_change: mpsc::Sender<CacheEvent>) -> Option<WatcherHandle> {
    let (shutdown_tx, shutdown_rx) = mpsc::channel();
    let thread = thread::Builder::new()
        .name("x11-watcher".into())
        .spawn(move || watcher_loop(on_change, shutdown_rx))
        .ok()?;
    Some(WatcherHandle {
        shutdown_tx: Some(shutdown_tx),
        thread: Some(thread),
    })
}

fn watcher_loop(on_change: mpsc::Sender<CacheEvent>, shutdown_rx: mpsc::Receiver<()>) {
    let Some((conn, atoms)) = connect_and_subscribe() else {
        return;
    };
    loop {
        match shutdown_rx.try_recv() {
            Ok(()) | Err(mpsc::TryRecvError::Disconnected) => {
                let _ = on_change.send(CacheEvent::Shutdown);
                return;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        match conn.poll_for_event() {
            Ok(Some(event)) => {
                if is_watched_property(&event, &atoms) {
                    drain_pending(&conn);
                    if on_change.send(CacheEvent::WindowsChanged).is_err() {
                        return;
                    }
                }
            }
            Ok(None) => thread::sleep(POLL_SLEEP),
            Err(_) => return,
        }
    }
}

fn connect_and_subscribe() -> Option<(x11rb::rust_connection::RustConnection, [u32; 3])> {
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots[screen_num].root;
    let atoms = intern_watched_atoms(&conn)?;
    let attrs = ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE);
    conn.change_window_attributes(root, &attrs).ok()?;
    conn.flush().ok()?;
    Some((conn, atoms))
}

fn intern_watched_atoms(conn: &impl Connection) -> Option<[u32; 3]> {
    let cookies: Vec<_> = WATCHED_ATOMS
        .iter()
        .map(|name| conn.intern_atom(false, name.as_bytes()))
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let mut atoms = [0u32; 3];
    for (i, cookie) in cookies.into_iter().enumerate() {
        atoms[i] = cookie.reply().ok()?.atom;
    }
    Some(atoms)
}

fn is_watched_property(event: &Event, atoms: &[u32; 3]) -> bool {
    if let Event::PropertyNotify(ev) = event {
        return atoms.contains(&ev.atom);
    }
    false
}

fn drain_pending(conn: &impl Connection) {
    while let Ok(Some(_)) = conn.poll_for_event() {}
}
