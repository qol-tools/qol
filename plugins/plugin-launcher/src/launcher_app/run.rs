use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};

use gpui::*;

use crate::daemon;
use crate::monitor::MonitorTracker;
use crate::platform;

use super::keepalive;
use super::windows::{activate_or_open_launcher, ActiveLaunchers};
use super::{LauncherView, PreloadedEntries, SharedEntries};

pub fn run() {
    let show_immediately = std::env::args().any(|a| a == "--show");
    eprintln!("[launcher] run start show_immediately={show_immediately}");

    if std::env::args().any(|a| a == "--kill") {
        daemon::send_kill();
        return;
    }

    if show_immediately && daemon::send_show() {
        return;
    }

    Application::new().run(move |cx: &mut App| {
        #[cfg(debug_assertions)]
        eprintln!("[launcher] run: pid={}", std::process::id());

        let focus_cache = MonitorTracker::start(cx);

        let (tx, rx) = mpsc::channel();
        if !daemon::start_listener(tx) {
            #[cfg(debug_assertions)]
            eprintln!("[launcher] daemon listener failed, quitting");
            cx.quit();
            return;
        }
        eprintln!("[launcher] daemon listener started");

        let entries: SharedEntries = Arc::new(Mutex::new(Arc::new(PreloadedEntries::empty())));
        let active: Rc<RefCell<ActiveLaunchers>> =
            Rc::new(RefCell::new(ActiveLaunchers::default()));

        keepalive::open_keepalive_window(cx);
        platform::set_activation_policy();

        spawn_command_poll(
            entries.clone(),
            active.clone(),
            rx,
            focus_cache.clone(),
            cx,
        );
        spawn_preload(entries.clone(), active.clone(), cx);

        if show_immediately {
            #[cfg(debug_assertions)]
            eprintln!("[launcher] show_immediately");
            let snapshot = focus_cache.snapshot();
            activate_or_open_launcher(entries.clone(), active.clone(), snapshot, cx);
        }
    });

    daemon::cleanup();
}

fn spawn_command_poll(
    entries: SharedEntries,
    active: Rc<RefCell<ActiveLaunchers>>,
    rx: mpsc::Receiver<daemon::Command>,
    focus_cache: MonitorTracker,
    cx: &mut App,
) {
    let rx = Arc::new(Mutex::new(rx));
    cx.spawn(async move |cx: &mut AsyncApp| {
        loop {
            let next_command = cx
                .background_spawn({
                    let rx = rx.clone();
                    async move {
                        let guard = rx.lock().ok()?;
                        guard.recv().ok()
                    }
                })
                .await;

            #[cfg(debug_assertions)]
            eprintln!(
                "[launcher] command_poll: next_command={}",
                match &next_command {
                    Some(daemon::Command::Show) => "Show",
                    Some(daemon::Command::Kill) => "Kill",
                    None => "None",
                }
            );
            match next_command {
                Some(daemon::Command::Show) => {
                    eprintln!("[launcher] command: show");
                    let focus_cache = focus_cache.clone();
                    eprintln!("[launcher] snapshot start");
                    let snapshot = cx
                        .background_spawn(async move { focus_cache.snapshot() })
                        .await;
                    eprintln!(
                        "[launcher] snapshot done: {}",
                        if snapshot.is_some() { "some" } else { "none" }
                    );
                    #[cfg(debug_assertions)]
                    eprintln!(
                        "[launcher] command_poll: snapshot={:?}",
                        snapshot.as_ref().map(|m| m.bounds())
                    );
                    let entries = entries.clone();
                    let active = active.clone();
                    eprintln!("[launcher] cx.update start");
                    if let Err(e) = cx.update(move |cx| {
                        activate_or_open_launcher(entries.clone(), active.clone(), snapshot, cx)
                    }) {
                        eprintln!("[launcher] command_poll: cx.update failed: {:?}", e);
                    } else {
                        eprintln!("[launcher] cx.update done");
                    }
                }
                Some(daemon::Command::Kill) => {
                    cx.update(|cx| cx.quit()).ok();
                    break;
                }
                None => break,
            }
        }
    })
    .detach();
}

fn spawn_preload(
    entries: SharedEntries,
    active: Rc<RefCell<ActiveLaunchers>>,
    cx: &mut App,
) {
    cx.spawn({
        let entries = entries.clone();
        let active = active.clone();
        async move |cx: &mut AsyncApp| {
            eprintln!("[launcher] preload start");
            let loaded = cx
                .background_spawn(async move { Arc::new(PreloadedEntries::load()) })
                .await;
            eprintln!("[launcher] preload done");
            if let Ok(mut guard) = entries.lock() {
                *guard = loaded.clone();
            }
            let _ = cx.update(move |cx| {
                let handles: Vec<WindowHandle<LauncherView>> =
                    active.borrow().handles();
                for handle in handles {
                    let loaded_entries = loaded.clone();
                    let _ = handle.update(cx, |view, _window, cx| {
                        view.store.replace_entries(
                            loaded_entries.app_entries.clone(),
                            loaded_entries.file_entries.clone(),
                        );
                        view.store.ensure_filtered(&view.state);
                        cx.notify();
                    });
                }
            });
        }
    })
    .detach();
}
