
## 2026-06-04
- On Linux X11, qol-tray's `is_ignored_pid` is gated `#[cfg(target_os = "macos")]` so the Linux focus query never consults it - daemon PIDs leak into `focused_window_bounds`.
- GPUI `WindowKind::PopUp` on X11 is NOT override-redirect; it sets `_NET_WM_WINDOW_TYPE_NOTIFICATION` and `activate_window()` sends `_NET_ACTIVE_WINDOW` + `set_input_focus`, so popups become the WM active window.
- Plugin daemons spawn via `command.spawn()` + `libc::setsid()` with a different PID than qol-tray, so `_NET_WM_PID == own_pid` checks won't catch picker windows.

## 2026-06-04
- In this monorepo, cargo package names don't match dir names: `plugins/plugin-alt-tab/` is package `alt-tab`; check `Cargo.toml` `name =` before `cargo -p`.
- Under `-D warnings`, test-only helpers tripped `dead_code`; gate pure invariant helpers behind `#[cfg(test)]` rather than `pub(crate)` when no production caller exists.
- GPUI X11 `set_bounds` drops origin on size-bearing ConfigureNotify, so `window.window_bounds()` reports stale origin after resize - don't trust it; use `translate_coordinates(wid, root, 0, 0)`.

## 2026-06-22
- Linux alt-tab filter rejects any window whose `_NET_WM_WINDOW_TYPE` is set but lacks `_NET_WM_WINDOW_TYPE_NORMAL` - GPUI PopUp = NOTIFICATION, so all GPUI popups vanish from the picker.
- cli-sessions panel uses `ghost_window_kind()` which is PopUp on Linux; real interactive panels should use `WindowKind::Normal` (focus + alt-tab visibility both depend on it).
- Linux discovery has no own-PID exclusion, no SKIP_TASKBAR check, no size filter, no map_state check - unlike macOS; widening the type allowlist risks surfacing internal keepalive ghosts.

## 2026-06-22
- Linux x11rb discovery reverses `_NET_CLIENT_LIST_STACKING` via `.iter().enumerate().rev()` in `collect_window_info`, so wire-level bottom-to-top becomes top-of-stack-first in the resulting Vec - don't trust "no reversal happens" at the call site.
- `_NET_WM_STATE_ABOVE` windows sit at the top of `_NET_CLIENT_LIST_STACKING`, so any always-on-top panel auto-pins to index 0 after the post-fetch `.rev()`, even before `_NET_ACTIVE_WINDOW` promotion runs.
- macOS alt-tab parity for popup panels lives in `true_focus_index` (layer-aware: promote popup-layer only if frontmost-app, else fall back to first NORMAL-layer) - Linux needs the same shape, not removal of always-on-top state.

## 2026-06-22
- Clippy `type_complexity` (`-D warnings`) fires on tuple slices like `&[(&str, &[u32], &[bool], Option<u32>, Option<usize>)]`; factor into a `type` alias before first compile.

## 2026-06-23
- `#[ignore]`-gated live X11 tests (`cargo test -- --ignored --nocapture`) catch real-session ordering bugs that pure unit tests miss.

## 2026-06-27
- When renaming a fn across cinnamon_shell/platform/mod/macos/windows/linux, also add `Debug` to any struct used in test `{:?}` format — `#[derive(Clone, Copy)]` alone fails compile after the test lands.
- `dismiss()` early-returns on `!is_active_visible()`, so any teardown that must run unconditionally (e.g. external preview-plane hide) belongs ABOVE the inactive guard, not after `PICKER_VISIBLE.store(false)`.
- Gate external-backend teardown calls on `self.rendering.preview_plane_backend().is_some()` rather than calling unconditionally — avoids spurious probe noise when no backend is active.

## 2026-06-27
- gdbus `call` needs `--timeout <s>` to bound hangs; `--session` alone won't cap wait time, even if a sibling `ping_available` already passes one.
