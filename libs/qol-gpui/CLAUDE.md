# qol-gpui

Shared GPUI helpers for qol-tray plugins: popup-window placement, ghost
keepalive, monitor tracking, runtime event routing.

## Opt-in X11 / window integration tests

The popup hide/show/configure code drives platform window state (X11 properties
on Linux, `NSWindow` on macOS) that a unit test cannot observe. Those paths are
covered by `#[ignore]` integration tests that assert the real window state
against a throwaway window they own, so they never touch the user's windows or
focus.

They need a real display + window manager, so they are ignored by default and
skip gracefully when `$DISPLAY` is unset:

```
cargo test -p qol-gpui --test popup_x11 -- --ignored
```

### Cross-platform coverage

These tests verify per-OS behavior, so a green run on one OS says nothing about
another. When you change shared popup code, re-run the matching test on each
platform you claim to support (or let a per-OS CI matrix run them).

| Platform | Integration test            | Status        |
| -------- | --------------------------- | ------------- |
| Linux    | `tests/popup_x11.rs`        | implemented   |
| macOS    | `tests/popup_macos.rs`      | TODO (mirror) |

The macOS mirror would assert `NSWindow` `alphaValue` and level instead of
`_NET_WM_WINDOW_OPACITY`, against the same `configure_popup_window` /
`hide_window_by_title` / `show_window_by_title` API.
