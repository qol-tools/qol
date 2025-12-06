# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build --release    # Build the launcher binary
cargo test               # Run unit tests
./tests/test_linux_backend.sh  # Run backend tests
```

After building, the binary is at `target/release/launcher`. The plugin directory is symlinked to `~/.config/qol-tray/plugins/plugin-launcher`, so rebuilding automatically updates the running qol-tray instance.

## Architecture

This is a qol-tray plugin providing a universal file search launcher using wry (WebView).

### Core Flow

1. qol-tray starts daemon via `[daemon]` config → WebView ready but hidden
2. User triggers hotkey → `run.sh` sends "show" via Unix socket → instant window display
3. User types query → JS sends IPC to Rust → background thread calls platform search backend
4. Results returned via `evaluate_script` → JS renders results
5. User selects result with modifier key → Rust records frequency, executes action → window hides

### Key Components

- `src/main.rs` - wry WebView setup, IPC handling, daemon mode, window positioning, frequency tracking
- `ui/` - Embedded at compile time via `include_str!()`
- `backends/` - Platform-specific search scripts (plocate, mdfind, Everything)
- `run.sh` - Triggers show via socket

### Key Functions

- `create_window()` - Window builder setup
- `create_ipc_handler()` - IPC message routing
- `show_window_linux()` - GTK focus/positioning logic
- `start_socket_listener()` - Unix socket for single instance

### IPC Protocol

JS → Rust messages (via `window.ipc.postMessage`):
```json
{"type": "search", "query": "..."}
{"type": "execute", "path": "...", "action": "open|terminal|folder|copy"}
{"type": "close"}
```

Rust → JS responses (via `evaluate_script`):
```js
window.onSearchResults([{path, name, is_dir}, ...])
```

### Actions

| Modifier | Action |
|----------|--------|
| Enter | Open file/directory |
| Ctrl+Enter | Open in terminal |
| Shift+Enter | Open containing folder |
| Alt+Enter | Copy path to clipboard |

### Dependencies

- `wry` - WebView (uses system WebKitGTK on Linux)
- `tao` - Window management
- `gtk` - Linux window positioning and focus
- `serde`/`serde_json` - IPC serialization

### Platform Backends

| Platform | Backend | Command |
|----------|---------|---------|
| Linux | plocate | `plocate --ignore-case --limit 200` + grep filtering |
| macOS | mdfind | `mdfind -name` |
| Windows | Everything CLI / Windows Search | `es.exe` or ADODB |

Linux backend supports custom plocate databases for mounted drives under `/media/`. Run `backends/update-dbs.sh` to index mounted drives.

## Code Style

- No comments in code
- Max 50 lines per function
- Max 2 levels of nesting
- Conventional commits: `feat:`, `fix:`, `refactor:`, `test:`
- Short commit messages, no co-authors
- AAA pattern for tests (Arrange/Act/Assert with comments)
- UI is embedded at compile time - changes require rebuild
