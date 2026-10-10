# Modifications

This is global-hotkey 0.7.0 from crates.io, changed by qol-tools:

- `src/platform_impl/windows/mod.rs`: one dedicated `global-hotkey` thread creates the hidden window, pumps its messages and runs every `RegisterHotKey` and `UnregisterHotKey`. `RegisterHotKey` binds a hotkey to the thread that owns the window, and upstream created the window on whichever thread built the manager, so hotkeys stopped firing when that thread did not pump messages. `register` and `unregister` send a request over a channel, wake the thread with a `WM_APP` message and wait for its reply. Dropping the manager posts `WM_CLOSE`, the window procedure answers `WM_DESTROY` with `PostQuitMessage`, and the drop joins the thread.
- `src/platform_impl/windows/mod.rs`: release detection is one 10 ms timer on the pump thread that polls `GetAsyncKeyState` for every held hotkey key and sends `Released` when its key is up, stopping once nothing is held. Upstream spawned a thread per press that spun on `GetAsyncKeyState` without sleeping and treated any nonzero state as still held. Unregistering a held hotkey sends its `Released` event so no press is left open.
