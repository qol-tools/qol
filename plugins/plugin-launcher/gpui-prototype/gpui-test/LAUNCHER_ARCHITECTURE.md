# Launcher GPUI Architecture

## Goal

Keep launcher behavior extensible without growing a single large module.

## Module Boundaries

- `src/bin/launcher.rs`
Runs the launcher by delegating to `gpui_test::launcher_app::run()`.

- `src/launcher_app/mod.rs`
Owns GPUI composition, event wiring, and coordination between state/search/view/actions.

- `src/launcher_app/state.rs`
Owns mutable launcher state:
`query`, caret, selection anchor, selected result index, current window height.
Also exposes text operations used by input and clipboard flows.

- `src/launcher_app/input.rs`
Maps `KeyDownEvent` to state transitions and high-level effects:
`Ignore`, `Notify`, `Launch`.

- `src/launcher_app/search.rs`
Pure filtering/ranking pipeline over desktop entries.
No window or rendering concerns.

- `src/launcher_app/layout.rs`
Window layout constants and resize logic.
Centralizes row count cap and height calculations.

- `src/launcher_app/view.rs`
Render helpers only:
search bar with caret/selection rendering and result rows with highlighted fuzzy matches.

- `src/launcher_app/actions.rs`
Launcher side effects (`spawn` selected command).

## Data Flow

1. Key event enters `mod.rs::handle_key`.
2. Clipboard shortcuts are handled first in controller layer.
3. Other keys flow through `state.apply_key(...)` from `input.rs`.
4. `search::filtered(...)` computes ranked results from current query.
5. `layout::resize_for_visible_rows(...)` updates popup height.
6. `view` renders current state and visible rows.

## Clipboard + Selection

Clipboard shortcuts target the query field only:
- `secondary+C`: copy selection
- `secondary+X`: cut selection
- `secondary+V`: paste text

`secondary` is platform aware via GPUI modifiers:
- macOS: `Cmd`
- Linux/Windows: `Ctrl`

## Extension Guidelines

- Add new pure query editing behavior in `state.rs` first.
- Add key mapping in `input.rs`.
- Keep platform/window/GPUI wiring in `mod.rs`.
- Keep rendering and styling in `view.rs`.
- Keep filesystem/process side effects in `actions.rs`.

This keeps tests and future feature branches isolated by concern.
