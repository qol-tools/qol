use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

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
        spawn_prewarm(entries.clone(), active.clone(), cx);

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

async fn wait_for_entries(entries: &SharedEntries, cx: &mut AsyncApp) {
    let executor = cx.background_executor().clone();
    loop {
        let ready = entries
            .lock()
            .map(|g| !g.app_entries.is_empty())
            .unwrap_or(false);
        if ready {
            break;
        }
        executor.timer(Duration::from_millis(50)).await;
    }
}

fn spawn_prewarm(
    entries: SharedEntries,
    active: Rc<RefCell<ActiveLaunchers>>,
    cx: &mut App,
) {
    cx.spawn({
        let entries = entries.clone();
        let active = active.clone();
        async move |cx: &mut AsyncApp| {
            let executor = cx.background_executor().clone();
            let mut warmed = false;
            loop {
                if warmed {
                    executor.timer(Duration::from_secs(2)).await;
                }

                // Skip if a launcher is currently visible
                let any_visible = cx
                    .update(|cx| {
                        active
                            .borrow()
                            .handles()
                            .iter()
                            .any(|h| h.update(cx, |v, _, _| v.is_showing).unwrap_or(false))
                    })
                    .unwrap_or(false);
                if any_visible {
                    warmed = true; // ensure timer fires on next iteration, no spin
                    continue;
                }

                let fresh = executor
                    .spawn(async { Arc::new(PreloadedEntries::load()) })
                    .await;

                if let Ok(mut guard) = entries.lock() {
                    *guard = fresh.clone();
                }

                let _ = cx.update(|cx| {
                    for handle in active.borrow().handles() {
                        let e = fresh.clone();
                        let _ = handle.update(cx, |view, _, cx| {
                            view.store.replace_entries(
                                e.app_entries.clone(),
                                e.file_entries.clone(),
                            );
                            view.store.ensure_filtered(&view.state);
                            cx.notify();
                        });
                    }
                });

                if !warmed {
                    eprintln!("[launcher] prewarm: initial load complete");
                }
                warmed = true;
            }
        }
    })
    .detach();
}
