# GPUI Test

GPUI development crate for the launcher.

## Linux Dependencies (Ubuntu/Debian)

```bash
sudo apt install gcc g++ libasound2-dev libfontconfig-dev libwayland-dev \
    libx11-xcb-dev libxkbcommon-x11-dev libssl-dev libzstd-dev libvulkan1 \
    libgit2-dev make cmake clang mold libstdc++-14-dev
```

Adjust `libstdc++-14-dev` based on your Ubuntu version:
- Ubuntu 24.04+: `libstdc++-14-dev`
- Ubuntu 22.04: `libstdc++-12-dev`
- Ubuntu 20.04: `libstdc++-10-dev`

## Build & Run

```bash
cargo run --bin launcher
```

## Launcher Behavior

- Borderless popup window anchored to active monitor
- Empty query shows only search bar (no suggestions)
- Results list grows dynamically by result count
- List height is capped at 8 rows
- Fuzzy ranking prioritizes contiguous matches

## Keyboard Controls

### Navigation

- `Esc`: close launcher
- `Up` / `Down`: move selected result
- `Enter`: launch selected result

### Query Editing

- `Left` / `Right`: move caret
- `Shift+Left` / `Shift+Right`: selection
- `Home` / `End`: move to start/end
- `Shift+Home` / `Shift+End`: select to start/end
- `Ctrl+A` (`Cmd+A` on macOS): select all
- `Backspace` / `Delete`: delete backward/forward

### Clipboard

- `Ctrl+C` (`Cmd+C` on macOS): copy selected query text
- `Ctrl+X` (`Cmd+X` on macOS): cut selected query text
- `Ctrl+V` (`Cmd+V` on macOS): paste clipboard text into query

`secondary` modifier is used internally:
- macOS: `Cmd`
- Linux/Windows: `Ctrl`

## Architecture

Entry point:
- `src/bin/launcher.rs` -> `gpui_test::launcher_app::run()`

Launcher modules:
- `src/launcher_app/mod.rs`: composition and GPUI event wiring
- `src/launcher_app/state.rs`: query/caret/selection state + text editing primitives
- `src/launcher_app/input.rs`: key handling and editing commands
- `src/launcher_app/search.rs`: pure filtering and fuzzy ranking
- `src/launcher_app/layout.rs`: window sizing constants and resize policy
- `src/launcher_app/view.rs`: search bar and result row rendering
- `src/launcher_app/actions.rs`: launch side effects on selected item
- `src/providers/apps/*`: app source abstraction and platform-specific app providers
- `src/providers/files/*`: file source abstraction and provider implementations
- `src/platform/mod.rs`: backend capability model for Linux display servers

Architecture policy:
- Every layer stays replaceable through explicit seams (`input`, `state`, `search`, `actions`, `providers`, `platform`).
- Platform and display-server specifics must stay behind adapters; launcher core remains OS-agnostic.
- If a feature cannot be added through a seam, introduce/refactor the seam first.

See `LAUNCHER_ARCHITECTURE.md` for details.
