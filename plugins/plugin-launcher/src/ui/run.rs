use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use gpui::*;

use crate::daemon;
use crate::discovery::{self, SharedEntries, SharedEntryState};
use crate::monitor::MonitorTracker;
use qol_gpui::command_loop::LoopFlow;

use super::keepalive;
use super::platform;
use super::window_host::{
    activate_or_open_launcher, pre_create_ghost, spawn_ghost_reposition_listener, ActiveLaunchers,
};

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

    let (tx, rx) = mpsc::channel();
    if !daemon::start_listener(tx) {
        eprintln!("[launcher] daemon listener failed, exiting");
        return;
    }
    eprintln!("[launcher] daemon listener started");

    Application::new().run(move |cx: &mut App| {
        #[cfg(debug_assertions)]
        eprintln!("[launcher] run: pid={}", std::process::id());

        let focus_cache = MonitorTracker::start(cx);

        let entries: SharedEntries = Arc::new(Mutex::new(SharedEntryState::pending()));
        let active: Rc<RefCell<ActiveLaunchers>> =
            Rc::new(RefCell::new(ActiveLaunchers::default()));

        keepalive::open_keepalive_window(cx);
        platform::set_activation_policy();

        crate::config::apply_ghost_debug();

        let boot_monitor = focus_cache.snapshot_monitor();
        pre_create_ghost(entries.clone(), active.clone(), boot_monitor, cx);
        spawn_ghost_reposition_listener(active.clone(), focus_cache.clone(), cx);

        spawn_command_poll(entries.clone(), active.clone(), rx, focus_cache.clone(), cx);
        discovery::start(entries.clone());

        if show_immediately {
            #[cfg(debug_assertions)]
            eprintln!("[launcher] show_immediately");
            cx.spawn({
                let entries = entries.clone();
                let active = active.clone();
                let focus_cache = focus_cache.clone();
                async move |cx: &mut AsyncApp| {
                    wait_for_entries(&entries, cx).await;
                    let snapshot = cx
                        .background_spawn(async move { focus_cache.snapshot().map(|(m, _)| m) })
                        .await;
                    let _ = cx.update(|cx| {
                        activate_or_open_launcher(entries, active, snapshot, cx);
                    });
                }
            })
            .detach();
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
    qol_gpui::command_loop::spawn_command_loop(cx, rx, move |cx, cmd| {
        let entries = entries.clone();
        let active = active.clone();
        let focus_cache = focus_cache.clone();
        async move {
            match cmd {
                daemon::Command::Show => {
                    wait_for_entries(&entries, &cx).await;
                    let snapshot = cx
                        .background_spawn(async move { focus_cache.snapshot().map(|(m, _)| m) })
                        .await;
                    let _ = cx
                        .update(move |cx| activate_or_open_launcher(entries, active, snapshot, cx));
                    LoopFlow::Continue
                }
                daemon::Command::Kill => LoopFlow::Stop,
            }
        }
    });
}

async fn wait_for_entries(entries: &SharedEntries, cx: &AsyncApp) {
    let executor = cx.background_executor().clone();
    loop {
        let ready = entries.lock().map(|g| g.loaded_once).unwrap_or(false);
        if ready {
            break;
        }
        executor.timer(Duration::from_millis(50)).await;
    }
}
