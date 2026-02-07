# Launcher GPUI Architecture

## Goal

Keep launcher behavior extensible without growing a single large module.

## Abstraction-First Policy

Every layer must be replaceable by introducing one more interface boundary.

Rules:
- No direct platform dependencies in launcher core modules.
- No direct provider implementation calls outside composition/factory boundaries.
- No feature may require touching more than one concern layer to add a new backend.
- New behavior must be introduced by adding an adapter at an existing seam, or by creating a new seam first.

Required seams:
- `input` seam: map raw keys to high-level intents.
- `state` seam: deterministic text/navigation transitions.
- `search` seam: pure ranking/filtering over provided data.
- `actions` seam: side effects for selected item execution.
- `providers` seam: data acquisition/indexing per platform/backend.
- `platform` seam: backend capability detection and backend-specific integrations.

Non-negotiable constraint:
- If a module cannot accept a new middle layer without rewrite, refactor before adding functionality.

## Module Boundaries

- `src/bin/launcher.rs`
Runs the launcher by delegating to `gpui_test::launcher_app::run()`.

- `src/launcher_app/mod.rs`
Owns GPUI composition, event wiring, and coordination between state/search/view/actions.
This is the only launcher boundary that chooses provider implementations.

- `src/launcher_app/state.rs`
Owns mutable launcher state:
`query`, caret, selection anchor, selected result index, current window height.
Also exposes text operations used by input and clipboard flows.

- `src/launcher_app/input.rs`
Maps `KeyDownEvent` to state transitions and high-level effects:
`Ignore`, `Notify`, `Launch`.

- `src/launcher_app/search.rs`
Pure filtering/ranking pipeline over in-memory entries.
No filesystem access, no provider selection, no window/rendering concerns.

- `src/launcher_app/layout.rs`
Window layout constants and resize logic.
Centralizes row count cap and height calculations.

- `src/launcher_app/view.rs`
Render helpers only:
search bar with caret/selection rendering and result rows with highlighted fuzzy matches.

- `src/launcher_app/actions.rs`
Launcher side effects (`spawn` selected command / open selected file path).
Consumes selected item directly from controller.

- `src/providers/files/mod.rs`
OS-agnostic file provider contract and default provider factory.

- `src/providers/files/fallback.rs`
Portable fallback file scanner used when no platform-specific indexed provider is active.

- `src/providers/linux/*` (planned)
Linux-specific provider implementations behind shared provider contracts.
Must isolate Wayland/X11/runtime distro differences from launcher core.

## Data Flow

1. Key event enters `mod.rs::handle_key`.
2. Clipboard shortcuts are handled first in controller layer.
3. Other keys flow through `state.apply_key(...)` from `input.rs`.
4. `search::filtered(...)` computes ranked results from current query.
5. `layout::resize_for_visible_rows(...)` updates popup height.
6. `view` renders current state and visible rows.
7. On launch, controller passes selected item to `actions` without recomputing search.

## Cross-Platform Guardrails

- Keep `launcher_app/*` OS-agnostic.
- Keep filesystem/index integration in `providers/files/*`.
- Keep provider selection at composition boundary (`launcher_app/mod.rs`) only.
- Avoid `#[cfg(...)]` inside launcher state/input/view/search modules.
- Keep display-server specifics (Wayland/X11) behind platform capability adapters.

## Capability Model

Platform adapters expose capabilities, not platform conditionals:
- `can_global_hotkey`
- `can_focus_popup`
- `can_clipboard_monitor`
- `can_window_positioning`

Feature code consumes capabilities and degrades gracefully when unavailable.
No feature module should branch on distro name or display server directly.

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
- Add new backend-specific logic in `providers/*` or `platform/*` adapters only.

## Entanglement Check Before Merge

A change must satisfy all checks:
- Core module compiles without referencing OS/backend crates.
- Backend/provider can be swapped by changing only factory/composition wiring.
- Search behavior remains testable with in-memory fixtures only.
- Action execution remains testable with selected-item inputs only.
- New mode/backend can be added without editing `view.rs` rendering primitives.

This keeps tests and future feature branches isolated by concern.
