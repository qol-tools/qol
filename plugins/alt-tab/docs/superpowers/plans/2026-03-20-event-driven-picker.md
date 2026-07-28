# Event-Driven Picker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the polling prewarm loop (3% idle CPU) with event-driven X11 property monitoring (0% idle CPU), and pre-create the picker window at daemon startup for instant (~1ms) open time.

**Architecture:** A dedicated background thread holds a persistent X11 connection, subscribes to `PropertyNotify` on the root window, and blocks on `wait_for_event()`. When `_NET_CLIENT_LIST_STACKING` or `_NET_ACTIVE_WINDOW` changes, it updates the shared window cache. The picker window is pre-created at daemon boot (offscreen, empty), so alt-tab only needs to update content and reposition — never recreate.

**Tech Stack:** x11rb 0.13.2 (`PropertyNotifyEvent`, `EventMask::PROPERTY_CHANGE`, `wait_for_event`), gpui 0.2.2 (`minimize_window`, `activate_window`).

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `src/discovery/watcher.rs` | **Create** | X11 event watcher: subscribes to root PropertyNotify, fires callback on stacking/active changes |
| `src/picker/run.rs` | **Modify** | Replace `spawn_prewarm_task` (polling loop) with `spawn_event_watcher` (blocking event thread). Pre-create picker at startup. |
| `src/picker/mod.rs` | **Modify** | Simplify `open_picker`: remove create path, always reuse pre-created window. Remove `ActiveWindows` map, use single `Option<WindowHandle>`. |
| `src/picker/create.rs` | **Modify** | Add `pre_create_picker` for offscreen empty window at boot. Keep `PickerInit` for content updates. |
| `src/picker/gather.rs` | **Modify** | Remove `cache_snapshot_matches` — cache is always fresh from event watcher. `gather` becomes a simple cache read. |
| `src/picker/reuse.rs` | **Modify** | Always-reuse path: reposition + resize + update content. No monitor-key lookup. |
| `src/picker/platform/linux.rs` | **Modify** | `dismiss_picker`: minimize (as now). Keep existing. |
| `src/app/mod.rs` | **Modify** | `apply_reuse` handles all opens (no more `new` path after boot). |
| `src/discovery/platform/linux.rs` | **Keep** | `on_screen_window_ids()`, `get_open_windows()` with `_NET_ACTIVE_WINDOW` promotion — already done. |
| `src/discovery/mod.rs` | **Modify** | Re-export watcher module. |

---

### Task 1: X11 Event Watcher Module

**Files:**
- Create: `src/discovery/watcher.rs`
- Modify: `src/discovery/mod.rs`

The watcher holds a persistent X11 connection, subscribes to PropertyNotify on root, and blocks on `wait_for_event()`. Zero CPU when idle.

- [ ] **Step 1: Create watcher module with types**

```rust
// src/discovery/watcher.rs
use std::sync::mpsc;
use std::thread;

pub enum CacheEvent {
    WindowsChanged,
    Shutdown,
}

pub struct WatcherHandle {
    tx_shutdown: mpsc::Sender<()>,
    join: Option<thread::JoinHandle<()>>,
}

impl WatcherHandle {
    pub fn stop(&mut self) {
        let _ = self.tx_shutdown.send(());
        if let Some(h) = self.join.take() {
            let _ = h.join();
        }
    }
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        self.stop();
    }
}
```

- [ ] **Step 2: Implement the event loop**

```rust
// src/discovery/watcher.rs (continued)
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;

pub fn spawn_watcher(on_change: mpsc::Sender<CacheEvent>) -> Option<WatcherHandle> {
    let (tx_shutdown, rx_shutdown) = mpsc::channel();
    let join = thread::Builder::new()
        .name("x11-watcher".into())
        .spawn(move || run_watcher_loop(on_change, rx_shutdown))
        .ok()?;
    Some(WatcherHandle {
        tx_shutdown,
        join: Some(join),
    })
}

fn run_watcher_loop(on_change: mpsc::Sender<CacheEvent>, rx_shutdown: mpsc::Receiver<()>) {
    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return;
    };
    let root = conn.setup().roots[screen_num].root;

    let stacking_atom = intern_atom(&conn, "_NET_CLIENT_LIST_STACKING");
    let active_atom = intern_atom(&conn, "_NET_ACTIVE_WINDOW");
    let client_atom = intern_atom(&conn, "_NET_CLIENT_LIST");

    let attrs = ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE);
    if conn.change_window_attributes(root, &attrs).is_err() {
        return;
    }
    let _ = conn.flush();

    loop {
        if rx_shutdown.try_recv().is_ok() {
            break;
        }
        let event = match conn.poll_for_event() {
            Ok(Some(event)) => event,
            Ok(None) => {
                // No event pending — wait with timeout so we can check shutdown
                std::thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }
            Err(_) => break,
        };
        if let Event::PropertyNotify(ev) = event {
            if ev.atom == stacking_atom
                || ev.atom == active_atom
                || ev.atom == client_atom
            {
                let _ = on_change.send(CacheEvent::WindowsChanged);
                // Drain any queued property events to avoid redundant refreshes
                drain_property_events(&conn);
            }
        }
    }
}

fn drain_property_events(conn: &impl Connection) {
    while let Ok(Some(Event::PropertyNotify(_))) = conn.poll_for_event() {}
}

fn intern_atom(conn: &impl Connection, name: &str) -> u32 {
    conn.intern_atom(false, name.as_bytes())
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.atom)
        .unwrap_or(0)
}
```

Note: Uses `poll_for_event` + 50ms sleep instead of `wait_for_event` so the shutdown channel can be checked. 50ms sleep only happens when no events are pending — near zero CPU. Alternative: use the X11 connection fd with `poll()` syscall + a pipe for shutdown signaling (more complex, truly 0 CPU). Start with the simple approach; optimize later if 50ms poll matters.

- [ ] **Step 3: Re-export from discovery/mod.rs**

Add to `src/discovery/mod.rs`:
```rust
pub(crate) mod watcher;
```

- [ ] **Step 4: Commit**

```bash
git add src/discovery/watcher.rs src/discovery/mod.rs
git commit -m "feat: add X11 event watcher for property change monitoring"
```

---

### Task 2: Replace Polling Prewarm with Event-Driven Cache Updates

**Files:**
- Modify: `src/picker/run.rs`

Replace `spawn_prewarm_task` (1200ms polling timer) with an event receiver that updates the cache only when the X11 watcher signals a change.

- [ ] **Step 1: Add watcher integration to run_app**

In `run_app` (line 76), replace `spawn_prewarm_task(cx, state.caches.clone())` with:

```rust
let (cache_tx, cache_rx) = std::sync::mpsc::channel();
let _watcher = crate::discovery::watcher::spawn_watcher(cache_tx);
spawn_cache_updater(cx, cache_rx, state.caches.clone());
// Do one immediate cache fill on startup
spawn_initial_cache_fill(cx, state.caches.clone());
```

- [ ] **Step 2: Implement spawn_cache_updater**

Replace the prewarm loop functions with:

```rust
fn spawn_cache_updater(
    cx: &mut App,
    rx: std::sync::mpsc::Receiver<crate::discovery::watcher::CacheEvent>,
    caches: PickerCaches,
) {
    let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
    cx.spawn(async move |cx: &mut AsyncApp| {
        let executor = cx.background_executor().clone();
        loop {
            let rx = rx.clone();
            let event = executor
                .spawn(async move { rx.lock().ok()?.recv().ok() })
                .await;
            let Some(event) = event else { break };
            match event {
                crate::discovery::watcher::CacheEvent::WindowsChanged => {
                    if picker_visible() {
                        continue;
                    }
                    refresh_cache(&executor, &caches).await;
                }
                crate::discovery::watcher::CacheEvent::Shutdown => break,
            }
        }
    })
    .detach();
}

async fn refresh_cache(executor: &gpui::BackgroundExecutor, caches: &PickerCaches) {
    let Some(windows) = load_stable_windows(executor, &caches.window_cache).await else {
        return;
    };
    caches
        .last_window_count
        .store(windows.len().max(1), std::sync::atomic::Ordering::Relaxed);
    refresh_icon_cache(executor, &windows, &caches.icon_cache).await;
    replace_window_cache(&caches.window_cache, windows);
}

fn spawn_initial_cache_fill(cx: &mut App, caches: PickerCaches) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        let executor = cx.background_executor().clone();
        refresh_cache(&executor, &caches).await;
    })
    .detach();
}
```

- [ ] **Step 3: Remove old prewarm functions**

Delete these functions from `run.rs`:
- `spawn_prewarm_task`
- `run_prewarm_loop`
- `wait_for_prewarm_tick`
- `should_skip_prewarm_refresh`
- `PrewarmState` struct
- `PREWARM_REFRESH_INTERVAL_MS` constant

Keep these (still used by `refresh_cache`):
- `load_stable_windows`
- `fetch_open_windows`
- `refresh_icon_cache`
- `cached_icon_names`, `missing_icon_windows`, `merge_icons`, `retain_active_icons`
- `replace_window_cache`, `read_window_cache`, `cached_window_len`
- `should_retry_small_result`, `choose_stable_windows`

- [ ] **Step 4: Store watcher handle to prevent drop**

In `run_app`, the `_watcher` variable must live for the lifetime of the app. It's already scoped to the `app.run()` closure. Ensure it's not dropped:

```rust
app.run(move |cx: &mut App| {
    // ... existing setup ...
    let (cache_tx, cache_rx) = std::sync::mpsc::channel();
    let _watcher = crate::discovery::watcher::spawn_watcher(cache_tx);
    spawn_cache_updater(cx, cache_rx, state.caches.clone());
    spawn_initial_cache_fill(cx, state.caches.clone());
    // ... rest of setup ...
    // _watcher lives until app.run() closure ends
});
```

- [ ] **Step 5: Commit**

```bash
git add src/picker/run.rs
git commit -m "feat: replace polling prewarm with event-driven cache updates"
```

---

### Task 3: Simplify Gather — Remove Cache Validation

**Files:**
- Modify: `src/picker/gather.rs`

The cache is now kept fresh by the event watcher. No need for the expensive `cache_snapshot_matches` validation at open time.

- [ ] **Step 1: Simplify windows_from_cache_or_discovery**

Replace the function:

```rust
fn windows_from_cache_or_discovery(
    config: &AltTabConfig,
    window_cache: &WindowCache,
) -> Vec<WindowInfo> {
    let cached = window_cache
        .lock()
        .ok()
        .map(|c| c.clone())
        .unwrap_or_default();
    if !cached.is_empty() {
        return apply_minimized_filter(config, cached);
    }
    recover_small_window_set(config, initial_display_windows(config))
}
```

- [ ] **Step 2: Remove cache_snapshot_matches and snapshot_matches_cached_visible_ids**

Delete these functions and the `#[cfg(test)] mod tests` block that tests them.

- [ ] **Step 3: Commit**

```bash
git add src/picker/gather.rs
git commit -m "refactor: remove cache validation — event watcher keeps cache fresh"
```

---

### Task 4: Pre-Create Picker Window at Daemon Startup

**Files:**
- Modify: `src/picker/run.rs`
- Modify: `src/picker/create.rs`
- Modify: `src/picker/mod.rs`

Create the picker window immediately at daemon boot (offscreen, with empty content) so the first alt-tab has zero creation overhead.

- [ ] **Step 1: Add pre_create_offscreen to create.rs**

```rust
pub(crate) fn pre_create_offscreen(
    config: &AltTabConfig,
    icon_cache: SharedIconCache,
    current: &PickerWindowState,
    cx: &mut App,
) {
    let gathered = GatheredWindows {
        windows: Vec::new(),
        previews: HashMap::new(),
        icons: HashMap::new(),
    };
    let init = PickerInit::new(config, gathered);
    let offscreen = Bounds {
        origin: point(px(-5000.0), px(-5000.0)),
        size: size(px(600.0), px(400.0)),
    };
    let Some(handle) = open_picker_window(offscreen, init, cx) else {
        return;
    };
    // Store with a sentinel key — will be replaced on first real open
    current.borrow_mut().insert(
        qol_plugin_api::window::MonitorKey {
            x: -5000,
            y: -5000,
            width: 600,
            height: 400,
        },
        handle,
    );
    // Immediately minimize so it doesn't flash
    let _ = handle.update(cx, |_, window, _| {
        window.minimize_window();
    });
}
```

- [ ] **Step 2: Call pre_create_offscreen in run_app**

In `run_app`, after creating PickerState and before spawning the daemon loop:

```rust
let boot_config = crate::config::load_alt_tab_config();
create::pre_create_offscreen(
    &boot_config,
    state.caches.icon_cache.clone(),
    &state.current,
    cx,
);
```

Add the necessary import at the top of `run.rs`:
```rust
use super::create;
```

- [ ] **Step 3: Verify open_picker always finds existing window**

With pre-creation, `try_reuse_existing` should always find a window via `any_existing()`. The create path (`create_from_request`) becomes a fallback only if the pre-created window was somehow lost. No changes needed here — the existing fallback is safe.

- [ ] **Step 4: Commit**

```bash
git add src/picker/run.rs src/picker/create.rs
git commit -m "perf: pre-create picker window at daemon boot for instant open"
```

---

### Task 5: Remove Old Prewarm Tests, Add Watcher Smoke Test

**Files:**
- Modify: `src/picker/run.rs` (tests module)
- Modify: `src/picker/gather.rs` (tests module)

- [ ] **Step 1: Update run.rs tests**

The `should_skip_refresh` tests are no longer relevant (function deleted). Remove the test module or replace with a test for the new cache refresh logic if applicable.

Keep the `should_retry_small_result` logic and any tests for `choose_stable_windows` if they exist (these are still used by `load_stable_windows`).

- [ ] **Step 2: Remove gather.rs snapshot tests**

The `snapshot_matches_cached_visible_ids` tests are no longer needed. Remove the `#[cfg(test)] mod tests` block in gather.rs.

- [ ] **Step 3: Commit**

```bash
git add src/picker/run.rs src/picker/gather.rs
git commit -m "test: remove obsolete prewarm and snapshot tests"
```

---

### Task 6: Clean Up — Remove Dead Code

**Files:**
- Modify: `src/discovery/platform/linux.rs`
- Modify: `src/picker/run.rs`

- [ ] **Step 1: Remove the debug eprintln from on_screen_window_ids**

The `[x11/snapshot]` log fires on every watcher event and from gather. Now that the system is event-driven and we have the `[alt-tab/cache]` log at open time, the per-snapshot log is unnecessary. Remove:

```rust
#[cfg(debug_assertions)]
eprintln!("[x11/snapshot] visible_ids (topmost-first): {:?}", result);
```

- [ ] **Step 2: Remove `should_skip_refresh` function and `SMALL_WINDOW_SET_MAX` / `STABLE_PREVIOUS_MIN` if only used by deleted prewarm code**

Check if these constants are still referenced. `SMALL_WINDOW_SET_MAX` is used by `should_retry_small_result` (still alive). `STABLE_PREVIOUS_MIN` is used by `should_retry_small_result` and `choose_stable_windows` (still alive). Keep both.

Remove only `should_skip_refresh` and its test if still present.

- [ ] **Step 3: Commit**

```bash
git add src/discovery/platform/linux.rs src/picker/run.rs
git commit -m "chore: remove dead prewarm code and noisy debug logs"
```

---

## Execution Notes

**Build verification after each task:** `cargo check` (not full build — fast feedback). Full `cargo build` and `cargo test` only after Task 5.

**Testing strategy:** The X11 watcher and GPUI pre-creation are integration-level — they need a running X server and GPUI context. Manual testing:
1. Start daemon: `cargo run`
2. Verify 0% CPU when idle: `top -p $(pgrep alt-tab)`
3. Open a new app → verify watcher fires (debug log)
4. Press alt-tab → verify instant open (no 2s delay)
5. Switch monitors → verify reuse (no recreation)

**Rollback:** If the event watcher approach has issues, the polling loop can be restored from git history. Each task is an atomic commit.
