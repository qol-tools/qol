---
name: hotkey-reload-must-reach-active-backend
description: hotkeys::trigger_reload() must dispatch to whichever capture backend is currently active; backend-owned RELOAD_SENDERs are silent-failure traps
metadata:
  type: feedback
---

When qol-tray has multiple hotkey backends (native CGEventTap on macOS, evdev on Linux, global_hotkey fallback), the shared `trigger_reload()` signal MUST live in a backend-agnostic module so whichever backend wins at startup is the one that gets the reload. A `RELOAD_SENDER: OnceLock<Sender<()>>` owned by a single backend's module is a silent-failure trap: the other backends will accept saves through the UI/HTTP layer but never apply them at runtime.

**Why:** A user reported that newly-bound hotkeys didn't take effect until full restart. Root cause: `RELOAD_SENDER` lived in `listener.rs` and only got `set()` when the global_hotkey listener path activated. On the active native path (the macOS + Linux default), it was never set, so `trigger_reload()` was a no-op. This violates the mission non-negotiable that failures must be visible — a silent save no-op is worse than a loud error.

**How to apply:**
- Cross-backend signal modules live in their own namespace (`hotkeys::reload`), with `subscribe()` returning a Receiver and `trigger_reload()` sending on the shared sender.
- Every backend's `install()` takes the `Receiver<()>` plus a `RebuildBindings: Box<dyn Fn() -> Vec<Binding> + Send + Sync>` callback, spawns its own reload thread, and swaps a shared matcher (`Arc<RwLock<...>>` or `Arc<Mutex<...>>`) on the runloop's behalf — without recreating the OS-level event tap.
- When adding a new cross-cutting plugin/runtime signal (reload, refresh, reconcile), audit ALL backends that might consume it. A signal that only one backend subscribes to is a bug, not a feature.
- Binding/build logic that initial-install and reload both use must be extracted into one helper (`build_capture_bindings`) so they can never drift.
