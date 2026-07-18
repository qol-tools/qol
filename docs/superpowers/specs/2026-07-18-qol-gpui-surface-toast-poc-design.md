# qol-gpui surface kit POC: qol-shot saved toast

## Context

Plugins hand-assemble their gpui windows from qol-gpui primitives (popup_window, event_router, monitor, ghost, keepalive, theme).
The long-term idea is a shared summonable-surface kit so every plugin surface shares window setup, placement, dismissal, and theming.
This spec covers only the first proof: one surface kind, one consumer.

## Goal

Introduce `qol_gpui::surface` with a single kind, `Toast`, and use it in qol-shot to show the "saved" confirmation instead of the host OS notification.
Success: after a screenshot saves, a themed toast appears on the cursor monitor over any app, activating it reveals the file, and it dismisses on timeout, all without taking focus from the user's current window.

## Non-goals

- `Palette`, `Picker`, `Hud` kinds.
- Migrating launcher, alt-tab, or qol-shot previews.
- The gpui component gallery.
- Any change to hotkeys, Shortcuts, or launcher export; those layers already work.

## Design

### `qol_gpui::surface` (new module)

```rust
pub enum SurfaceKind { Toast }

pub enum Anchor { CornerStack(Corner) }

Surface::new(SurfaceKind::Toast)
    .title("qol-shot saved")
    .anchor(Anchor::CornerStack(Corner::BottomRight))
    .timeout(Duration::from_millis(8_000))
    .show(cx, |window, cx| ToastView::new(...))
```

The builder owns:

- window options: never-focus configuration, `WindowKind` and override-redirect handling per platform, compositor-bypass awareness so the toast renders over fullscreen apps
- placement: anchor resolved against `MonitorBounds` for the cursor monitor at show-time
- dismissal: timeout, plus explicit dismiss from the view (activation counts as dismiss)
- theming: `qol_theme` tokens for surface color, elevation, radius

Focus-taking behavior (dismiss stacks, focus reassert) is explicitly out of the module until a focus-taking kind lands.

### qol-shot integration

- `completion.rs` gains a toast path beside `platform::show_saved_notification`.
- A `qol-config.toml` select field `saved_feedback` chooses `notification` (default, current behavior) or `toast`.
- The toast renders in the existing qol-shot gpui daemon process; no new binary.
- Toast content: text-only (file name and a saved label); activation triggers the existing `RevealTarget` reveal. A thumbnail is a later iteration, not part of the POC.
- Activation is mouse click; the toast never takes keyboard focus, so keyboard reveal stays on the notification path for now.

## Error handling

- If the daemon cannot create the toast window, fall back to `show_saved_notification` and log the failure; saving must never lose its confirmation.
- Timeout and reveal use the existing `RevealTarget` open-once guard, so double activation cannot reveal twice.

## Testing

- Unit tests in qol-gpui: anchor resolution against sample `MonitorBounds`, timeout policy state transitions.
- qol-shot contract test updates for the new config field.
- Runtime verification in a `qol env up` guest: toast over a normal window, over a fullscreen app, activation reveal, timeout dismiss, fallback when window creation fails.

## Extraction rule

Nothing else moves into `surface` until a second consumer needs it.
The next kind is only added together with its first consumer.
