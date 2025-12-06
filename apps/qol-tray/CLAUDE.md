# CLAUDE.md

## IMPORTANT: Linux Only (For Now)

Cross-platform support is planned for the future, but **not now**. Do NOT implement macOS or Windows code until explicitly asked. Do NOT add cross-platform CI workflows or abstractions. Focus only on Linux. When the user asks for cross-platform support, then implement it.

## Development Commands

```bash
make run      # Build and run
make dev      # Build and run with dev features (Developer tab)
make test     # Run tests
make install  # Build release and install to /usr/bin
make clean    # Clean build artifacts
make deb      # Build .deb package
make release  # Bump version, build, push, create GitHub release
```

## Architecture

**Minimal tray menu:** The tray menu only has "Plugins" (opens browser UI) and "Quit". All plugin interaction happens in the browser.

### Core Modules

**src/plugins/** - Plugin loading, execution, and configuration
- Scans `~/.config/qol-tray/plugins/` for plugin directories
- Each plugin has: `plugin.toml` (manifest), `run.sh` (executable), optional `config.json`
- Supports daemon processes and config toggles
- Key types: `Plugin`, `PluginManager`, `PluginManifest`
- Files: `mod.rs` (Plugin struct), `manager.rs` (PluginManager), `loader.rs` (scan/load), `manifest.rs` (data structures)

**src/menu/** - Menu abstraction and event routing
- `builder.rs`: Builds minimal menu (features + Quit), no per-plugin items
- `router.rs`: EventRouter with EventPattern (Exact/Prefix) for O(k) routing
- EventHandler supports Sync/Async execution
- Event format: `feature-id::menu-item-id`

**src/tray/** - System tray UI with platform abstraction
- Platform-specific implementations in `platform/` subdirectory
  - `linux.rs`: GTK-based, spawns separate thread for event loop
  - `windows.rs`, `macos.rs`: Standard tray icon implementation
- `PlatformTray` enum handles platform differences at compile time
- `icon.rs`: Icon loading from embedded RGBA data, supports notification dot variant

**src/features/plugin_store/** - Browser-based plugin management
- Serves web UI at `http://127.0.0.1:42700`
- Landing page shows installed plugins and plugin store
- Plugin settings accessed via `/plugins/{plugin_id}/`
- API endpoints for install/uninstall operations
- Fetches available plugins from `github.com/qol-tools/*`

**src/updates/** - Auto-update system
- Checks GitHub API on startup for new releases (2s timeout)
- Compares semantic versions
- Shows orange notification dot on tray icon when update available
- Menu item "⬆ Update to vX.Y.Z" downloads .deb and installs via `pkexec dpkg -i`
- Kills plugin daemons before restart to avoid socket conflicts

### Plugin Manifest Format

Plugins define their menu structure in `plugin.toml`:

```toml
[plugin]
name = "Plugin Name"
description = "Description"
version = "1.0.0"

[menu]
label = "Menu Label"
items = [
    { type = "action", id = "run", label = "Run", action = "run" },
    { type = "checkbox", id = "toggle", label = "Enable", checked = true,
      action = "toggle-config", config_key = "enabled" },
    { type = "separator" },
    { type = "submenu", id = "sub", label = "More", items = [...] }
]

[daemon]  # Optional
enabled = true
command = "daemon.sh"
restart_on_crash = false
```

Action types:
- `run` - Execute `run.sh`
- `toggle-config` - Toggle boolean in `config.json` at `config_key` path
- `settings` - Reserved for future use

### Code Style

- **No comments** - Code removed all comments; keep it that way
- **Conventional commits** - Use format: `feat:`, `fix:`, `refactor:`, `test:`, etc.
- **Short commit messages** - One-liners, no fluff, no co-authors
- **Atomic commits** - One logical change per commit. Split distinct changes (bug fix, refactor, tests) into separate commits. Each commit must compile and represent a working state.
- **No dead code warnings** - Remove unused code or gate with feature flags
- Platform-specific code belongs in `platform/` directories, not root modules
- **No builds or tests unless asked** - Do not run `cargo build`, `cargo run`, `make`, or browser tests unless explicitly requested. These operations are expensive.

### Single Responsibility Patterns

- **Describe without AND** - If you need "and" to describe a function, split it
- **Extract by abstraction level** - High-level orchestration shouldn't contain low-level details
- **Input → Transform → Output** - Functions should be one of: gather input, transform data, produce output. Don't mix I/O with business logic.
- **Command/Query separation** - Functions either change state OR return data, not both

### Type Safety Patterns

- **Newtypes for domain concepts** - Use `struct PluginId(String)` not raw `String`
- **Make invalid states unrepresentable** - Use enums to model state machines, not bool flags with optional fields
- **Parse, don't validate** - Parse into validated types at boundaries, use those types internally
- **Exhaustive matching** - Always match all enum variants explicitly (no `_ =>`), compiler catches new variants

### Frontend Architecture

- **Functional and declarative** - Pure render functions, no imperative DOM manipulation
- **Data-driven** - UI derived from state, not manually synchronized
- **Single responsibility** - Split logical chunks into focused modules
- **Type safety** - Define data structures explicitly, validate API responses
- **Scalability** - Design for N plugins, not hardcoded assumptions
- **Keyboard-first** - All interactions MUST be accessible via keyboard. This is critical. Design keyboard flow first, then add mouse/hover as secondary. Use single-letter shortcuts (e.g., `d` for delete) since Mac lacks Delete key. Always show keyboard hints in UI.

### Complexity Thresholds (Deep Modules Philosophy)

Inspired by "A Philosophy of Software Design" by John Ousterhout:

- **Deep modules over shallow** - Hide complexity behind simple, clean APIs. A function should do meaningful work, not just delegate. Prefer fewer functions that do more over many trivial wrappers.
- **Max 50 lines per function** - Split beyond this, but only if it creates genuinely reusable abstractions
- **Nesting is acceptable** - Common idioms like `for` + `if`, `match` in loop, early returns are fine. Extract helpers only when it genuinely clarifies intent or creates reusable logic.
- **One concern per function** - Don't mix state management, navigation, and action dispatch
- **Avoid shallow extractions** - Don't create `ensure_parent_dir()` if it's only called once and the inline version is equally clear. Extract when the abstraction has a meaningful name and hides real complexity.
- **Clean interfaces** - Public APIs should be obvious and hard to misuse. Internal complexity is fine if the interface is clean.

Frontend-specific:
- **Sequential ifs checking selectors** → Use config array with `{ selector, handler }` objects
- **Conditional rendering with shared structure** → Extract state-specific render functions
- **Key event handlers** → Separate recording/navigation/actions, use declarative handler maps

### Test Style

- **Table-driven tests** - Consolidate similar test cases into a single test with a cases array:
  ```rust
  let cases = [("input1", expected1), ("input2", expected2)];
  for (input, expected) in cases {
      assert_eq!(func(input), expected, "input: {}", input);
  }
  ```
- **Context in assertions** - Always include identifying info in assertion messages for debugging failed iterations
- **AAA pattern** - Use Arrange/Act/Assert comments for larger, complex tests where structure aids clarity. Omit for table-driven tests and simple one-liner tests where AAA would be redundant.
- **Generic test data** - Use abstract paths like `/a/b/c/foo` not personal-looking paths like `/home/user/documents/file.txt`. Use generic names (`foo`, `bar`) not real app names (`firefox`, `discord`).
- **No tests for thin wrappers** - If a function just calls already-tested functions, don't test it separately. Example: `fn foo(x) { bar(x).baz() }` doesn't need tests if `bar()` and `baz()` are tested.
- **Meaningful assertions** - Tests must verify specific behavior. Never just assert `result.is_ok()` - check the actual value.
- **Skip trivial Arrange** - For simple inputs, inline them. Only use explicit Arrange section for complex setup.
- **Descriptive names** - Use snake_case that explains what is being tested: `version_parsing_extracts_parts` not `test_parse`

## Icon Management

Icon is embedded as raw RGBA data at compile time from `assets/icon.rgba` (64x64 pixels, generated from `icon.png`).

To update icon:
1. Edit `assets/icon.png`
2. Convert to RGBA: `python3 -c "from PIL import Image; img = Image.open('assets/icon.png'); open('assets/icon.rgba', 'wb').write(img.tobytes())"`
3. Rebuild

## Plugin Development

Plugins are external to this codebase. They live in `~/.config/qol-tray/plugins/`.

The daemon provides:
- Plugin loading and manifest parsing
- Browser-based settings UI (each plugin can have `ui/index.html`)
- Config file management (read/write JSON)
- Process execution (scripts and daemons)

Plugins handle their own logic via shell scripts.
