# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build --release    # Build the launcher binary
cargo build              # Debug build
```

After building, the binary is at `target/release/launcher`. The plugin directory is symlinked to `~/.config/qol-tray/plugins/plugin-launcher`, so rebuilding automatically updates the running qol-tray instance.

## Architecture

This is a qol-tray plugin providing a universal file search launcher using wry (WebView).

### Core Flow

1. User triggers hotkey → qol-tray runs `run.sh` → launches `launcher` binary
2. Binary spawns borderless WebView window with embedded HTML/CSS/JS
3. User types query → JS sends IPC to Rust → Rust calls platform search backend
4. Results returned via `evaluate_script` → JS renders results
5. User selects result with modifier key → Rust executes action → window closes

### Key Components

- `src/main.rs` - wry WebView setup, IPC handling, action execution
- `ui/` - Embedded at compile time via `include_str!()`
- `backends/` - Platform-specific search scripts (plocate, mdfind, Everything)

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
- `serde`/`serde_json` - IPC serialization

### Platform Backends

| Platform | Backend | Command |
|----------|---------|---------|
| Linux | plocate | `plocate --ignore-case --limit 50` |
| macOS | mdfind | `mdfind -name` |
| Windows | Everything CLI / Windows Search | `es.exe` or ADODB |

## Code Style

- No comments in code
- Conventional commits: `feat:`, `fix:`, `refactor:`
- Short commit messages, no co-authors
- UI is embedded at compile time - changes require rebuild
