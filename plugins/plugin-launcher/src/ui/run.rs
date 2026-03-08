use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use gpui::*;

use crate::daemon;
use crate::discovery::{self, SharedEntries, SharedEntryState};
use crate::monitor::MonitorTracker;

use super::keepalive;
use super::platform;
use super::windows::{activate_or_open_launcher, ActiveLaunchers};

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

        let entries: SharedEntries = Arc::new(Mutex::new(SharedEntryState::pending()));
        let active: Rc<RefCell<ActiveLaunchers>> =
            Rc::new(RefCell::new(ActiveLaunchers::default()));

        keepalive::open_keepalive_window(cx);
        platform::set_activation_policy();

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
    let rx = Arc::new(Mutex::new(rx));
    cx.spawn(async move |cx: &mut AsyncApp| loop {
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
                wait_for_entries(&entries, cx).await;
                let focus_cache = focus_cache.clone();
                eprintln!("[launcher] snapshot start");
                let snapshot = cx
                    .background_spawn(async move { focus_cache.snapshot().map(|(m, _)| m) })
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
                let show_entries = entries.clone();
                let active = active.clone();
                eprintln!("[launcher] cx.update start");
                match cx
                    .update(move |cx| activate_or_open_launcher(show_entries, active, snapshot, cx))
                {
                    Ok(_) => eprintln!("[launcher] cx.update done"),
                    Err(e) => eprintln!("[launcher] command_poll: cx.update failed: {:?}", e),
                }
            }
            Some(daemon::Command::Kill) => {
                cx.update(|cx| cx.quit()).ok();
                break;
            }
            None => break,
        }
    })
    .detach();
}

async fn wait_for_entries(entries: &SharedEntries, cx: &mut AsyncApp) {
    let executor = cx.background_executor().clone();
    loop {
        let ready = entries.lock().map(|g| g.loaded_once).unwrap_or(false);
        if ready {
            break;
        }
        executor.timer(Duration::from_millis(50)).await;
    }
}
