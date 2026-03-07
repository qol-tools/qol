# Centralized Logging Module Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Unify all log control logic into `src/logging/`, add runtime-filterable core log controls via `FilterableLogger`, and expose them on the dev page.

**Architecture:** Move plugin log control persistence and relay into `src/logging/`. Wrap env_logger with a `FilterableLogger` that checks `Arc<RwLock<HashMap<String, LogControl>>>` for per-section mute/suppress before delegating. New API endpoints and UI section mirror the existing plugin log control pattern.

**Tech Stack:** Rust (log, env_logger, axum, serde), vanilla JS (frontend)

---

## Parallelization Strategy

```
Phase 1: Foundation (sequential, main branch)
    |
    v merge
Phase 2: Three parallel worktrees
    |-- Stream A: FilterableLogger + main.rs wiring
    |-- Stream B: Core persistence + API endpoints
    |-- Stream C: Frontend UI
    |
    v merge all
Phase 3: Integration verification
```

---

## Phase 1: Foundation (Sequential)

Everything else depends on this. Must complete and merge before Phase 2 begins.

### Task 1: Create logging module, move control.rs

**Files:**
- Create: `src/logging/mod.rs`
- Move: `src/plugins/log_control.rs` -> `src/logging/control.rs`
- Modify: `src/lib.rs`
- Modify: `src/plugins/mod.rs`

**Step 1: Create directory and move file**

```bash
mkdir -p src/logging
mv src/plugins/log_control.rs src/logging/control.rs
```

**Step 2: Create `src/logging/mod.rs`**

```rust
mod control;

pub use control::{
    load_all_plugin_controls, load_plugin_control, load_plugin_control_from_shared_config,
    normalize_patterns, save_all_plugin_controls, upsert_plugin_control, LogControl,
};
```

**Step 3: Register module in `src/lib.rs`**

Add between `pub mod installer;` and `pub mod menu;` (alphabetical):

```rust
pub mod logging;
```

**Step 4: Remove old module from `src/plugins/mod.rs`**

Delete the line `pub mod log_control;`

**Step 5: Rename type and functions in `src/logging/control.rs`**

| Old name | New name |
|---|---|
| `PluginLogControl` | `LogControl` |
| `PluginLogControlState` | `PluginLogControlFile` |
| `load_all_controls` | `load_all_plugin_controls` |
| `save_all_controls` | `save_all_plugin_controls` |
| `load_control` | `load_plugin_control` |
| `load_control_from_shared_config` | `load_plugin_control_from_shared_config` |
| `upsert_control` | `upsert_plugin_control` |

Update all internal references within the file (function calls, struct constructors, test code).
The `PluginLogControlFile` serde wrapper keeps its `plugins` field name unchanged — JSON format is backward compatible.

**Step 6: Add shared suppress helper to `src/logging/control.rs`**

```rust
pub(crate) fn matches_any_pattern(text: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| text.contains(p.as_str()))
}
```

### Task 2: Move and decouple relay.rs

**Files:**
- Move: `src/plugins/daemon_lifecycle/log_relay.rs` -> `src/logging/relay.rs`
- Modify: `src/plugins/daemon_lifecycle/mod.rs`
- Modify: `src/logging/mod.rs`

**Step 1: Move file**

```bash
mv src/plugins/daemon_lifecycle/log_relay.rs src/logging/relay.rs
```

**Step 2: Remove old module from `src/plugins/daemon_lifecycle/mod.rs`**

Delete the line `mod log_relay;`

**Step 3: Add relay to `src/logging/mod.rs`**

```rust
pub(crate) mod relay;
```

**Step 4: Rewrite `src/logging/relay.rs` — decouple from Plugin/Child**

```rust
use std::io::{BufRead, BufReader, Read};
use std::sync::Arc;

pub(crate) fn attach(
    label: &str,
    stdout: Option<impl Read + Send + 'static>,
    stderr: Option<impl Read + Send + 'static>,
    suppress_patterns: Vec<String>,
) {
    let patterns = active_patterns(suppress_patterns);
    if let Some(stdout) = stdout {
        spawn_relay(label.to_owned(), "stdout", stdout, patterns.clone(), false);
    }
    if let Some(stderr) = stderr {
        spawn_relay(label.to_owned(), "stderr", stderr, patterns, true);
    }
}

fn active_patterns(patterns: Vec<String>) -> Option<Arc<Vec<String>>> {
    let active: Vec<String> = patterns.into_iter().filter(|p| !p.is_empty()).collect();
    if active.is_empty() {
        return None;
    }
    Some(Arc::new(active))
}

fn spawn_relay<R: Read + Send + 'static>(
    label: String,
    stream_name: &'static str,
    reader: R,
    suppress_patterns: Option<Arc<Vec<String>>>,
    to_stderr: bool,
) {
    std::thread::spawn(move || {
        let prefix = format!("{} ({})", label, stream_name);
        relay_lines(reader, &prefix, suppress_patterns.as_ref(), to_stderr);
    });
}

fn relay_lines(
    reader: impl Read,
    prefix: &str,
    suppress: Option<&Arc<Vec<String>>>,
    to_stderr: bool,
) {
    let mut buf = BufReader::new(reader);
    let mut line = String::new();
    loop {
        line.clear();
        let n = buf.read_line(&mut line).unwrap_or_else(|e| {
            log::debug!("Log relay failed for {}: {}", prefix, e);
            0
        });
        if n == 0 {
            break;
        }
        if !should_suppress(&line, suppress) {
            if to_stderr {
                eprint!("{}", line);
            } else {
                print!("{}", line);
            }
        }
    }
}

fn should_suppress(line: &str, patterns: Option<&Arc<Vec<String>>>) -> bool {
    let Some(patterns) = patterns else {
        return false;
    };
    super::control::matches_any_pattern(line.trim_end(), patterns)
}
```

### Task 3: Update all import paths across codebase

**Files to modify:**
- `src/plugins/daemon_lifecycle/spawn.rs`
- `src/dev/linking/listing.rs`
- `src/features/plugin_store/server/dev_link_handlers.rs`

**Step 1: Update `src/plugins/daemon_lifecycle/spawn.rs`**

Replace:
```rust
use super::log_relay;
```
With: remove this import entirely.

Replace the call at the end of `spawn_daemon`:
```rust
// Before:
log_relay::attach_filtered_log_relay(plugin, &mut child, relay_patterns);

// After:
crate::logging::relay::attach(
    &plugin.id,
    child.stdout.take(),
    child.stderr.take(),
    relay_patterns,
);
```

Replace:
```rust
let log_control = crate::plugins::log_control::load_control_from_shared_config(&plugin.id);
```
With:
```rust
let log_control = crate::logging::load_plugin_control_from_shared_config(&plugin.id);
```

**Step 2: Update `src/dev/linking/listing.rs`**

Replace:
```rust
let log_controls = crate::plugins::log_control::load_all_controls(config_dir);
```
With:
```rust
let log_controls = crate::logging::load_all_plugin_controls(config_dir);
```

Replace:
```rust
log_control: crate::plugins::log_control::PluginLogControl,
```
With:
```rust
log_control: crate::logging::LogControl,
```

**Step 3: Update `src/features/plugin_store/server/dev_link_handlers.rs`**

Replace all `crate::plugins::log_control::PluginLogControl` with `crate::logging::LogControl`.
Replace `crate::plugins::log_control::upsert_control` with `crate::logging::upsert_plugin_control`.
Replace `crate::plugins::log_control::load_all_controls` with `crate::logging::load_all_plugin_controls`.

### Task 4: Verify and commit

**Step 1: Verify compilation**

```bash
cargo check --all-features
```

Expected: compiles with zero errors and no new warnings.

**Step 2: Commit**

```bash
git add -A
git commit -m "refactor: move log control and relay into src/logging/"
```

---

## Phase 2A: FilterableLogger (Parallel Worktree)

Branch from Phase 1 merge commit. Independent of Phase 2B and 2C.

### Task 5: Write FilterableLogger

**Files:**
- Create: `src/logging/filter.rs`
- Modify: `src/logging/mod.rs`

**Step 1: Add module to `src/logging/mod.rs`**

Add under the existing declarations, dev-gated:

```rust
#[cfg(feature = "dev")]
mod filter;

#[cfg(feature = "dev")]
pub use filter::init_filterable_logger;
```

**Step 2: Create `src/logging/filter.rs`**

```rust
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use super::LogControl;

pub type CoreControlsHandle = Arc<RwLock<HashMap<String, LogControl>>>;

struct Section {
    name: &'static str,
    prefix: &'static str,
}

const SECTIONS: &[Section] = &[
    Section { name: "runtime", prefix: "qol_tray::runtime" },
    Section { name: "plugins", prefix: "qol_tray::plugins" },
];

const CATCH_ALL: &str = "core";

fn section_for_target(target: &str) -> Option<&'static str> {
    for section in SECTIONS {
        if target.starts_with(section.prefix) {
            return Some(section.name);
        }
    }
    if target.starts_with("qol_tray") {
        return Some(CATCH_ALL);
    }
    None
}

struct FilterableLogger {
    inner: env_logger::Logger,
    controls: CoreControlsHandle,
}

impl log::Log for FilterableLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &log::Record) {
        if !self.inner.enabled(record.metadata()) {
            return;
        }
        if self.is_suppressed(record) {
            return;
        }
        self.inner.log(record);
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

impl FilterableLogger {
    fn is_suppressed(&self, record: &log::Record) -> bool {
        let Some(section) = section_for_target(record.target()) else {
            return false;
        };
        let controls = self.controls.read().unwrap_or_else(|e| e.into_inner());
        let Some(control) = controls.get(section) else {
            return false;
        };
        if control.muted {
            return true;
        }
        if control.suppress_patterns.is_empty() {
            return false;
        }
        let message = record.args().to_string();
        super::control::matches_any_pattern(&message, &control.suppress_patterns)
    }
}

pub fn init_filterable_logger(controls: CoreControlsHandle) {
    let inner = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    )
    .build();
    let max_level = inner.filter();
    let logger = FilterableLogger { inner, controls };
    log::set_boxed_logger(Box::new(logger))
        .expect("Logger already initialized");
    log::set_max_level(max_level);
}
```

### Task 6: Wire FilterableLogger into main.rs

**Files:**
- Modify: `src/main.rs`

**Step 1: Replace logger initialization in `main.rs`**

Replace line 16:
```rust
env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
```

With:
```rust
#[cfg(feature = "dev")]
let core_log_controls = {
    let controls = qol_tray::logging::init_dev_logger();
    controls
};

#[cfg(not(feature = "dev"))]
env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
```

**Step 2: Add `init_dev_logger` to `src/logging/mod.rs`**

This is a convenience function that loads controls from disk and initializes the logger:

```rust
#[cfg(feature = "dev")]
pub fn init_dev_logger() -> filter::CoreControlsHandle {
    let controls = load_core_controls_from_shared_config();
    let handle = std::sync::Arc::new(std::sync::RwLock::new(controls));
    filter::init_filterable_logger(handle.clone());
    handle
}

#[cfg(feature = "dev")]
fn load_core_controls_from_shared_config() -> std::collections::HashMap<String, LogControl> {
    crate::paths::shared_config_dir()
        .map(|dir| control::load_all_core_controls(&dir))
        .unwrap_or_default()
}
```

Note: `load_all_core_controls` is implemented in Phase 2B Task 7. If building Stream A in isolation, stub it to return an empty HashMap.

**Step 3: Thread controls handle into async_init and AppState**

In `main.rs`, pass `core_log_controls` through `async_init` → `start_ui_server`.
This requires adding a parameter to `async_init`:

```rust
// main.rs fn app_init()
#[cfg(feature = "dev")]
let init = rt.block_on(async_init(core_log_controls))?;

#[cfg(not(feature = "dev"))]
let init = rt.block_on(async_init())?;
```

Add to `InitResult`:
```rust
#[cfg(feature = "dev")]
core_log_controls: qol_tray::logging::CoreControlsHandle,
```

Add to `async_init` signature and pass through to `InitResult`.

The `CoreControlsHandle` type alias must be re-exported from `src/logging/mod.rs`:

```rust
#[cfg(feature = "dev")]
pub use filter::CoreControlsHandle;
```

### Task 7: Verify and commit

```bash
cargo check --all-features
cargo check
git add -A
git commit -m "feat: add FilterableLogger wrapping env_logger for dev builds"
```

---

## Phase 2B: Core Control Persistence + API (Parallel Worktree)

Branch from Phase 1 merge commit. Independent of Phase 2A and 2C.

### Task 7: Add core control persistence

**Files:**
- Modify: `src/logging/control.rs`
- Modify: `src/logging/mod.rs`

**Step 1: Add core control file constant and serde wrapper to `src/logging/control.rs`**

```rust
const CORE_LOG_CONTROL_STATE_FILE: &str = "dev-core-log-controls.json";

#[cfg(feature = "dev")]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CoreLogControlFile {
    #[serde(default)]
    sections: HashMap<String, LogControl>,
}
```

**Step 2: Add core control load/save functions to `src/logging/control.rs`**

```rust
#[cfg(feature = "dev")]
pub fn load_all_core_controls(config_dir: &Path) -> HashMap<String, LogControl> {
    let path = config_dir.join(CORE_LOG_CONTROL_STATE_FILE);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };
    serde_json::from_str::<CoreLogControlFile>(&content)
        .map(|state| state.sections)
        .unwrap_or_default()
}

#[cfg(feature = "dev")]
fn save_all_core_controls(
    config_dir: &Path,
    controls: &HashMap<String, LogControl>,
) -> Result<(), String> {
    if let Err(e) = std::fs::create_dir_all(config_dir) {
        return Err(format!(
            "Failed to create config directory {}: {}",
            config_dir.display(),
            e
        ));
    }
    let path = config_dir.join(CORE_LOG_CONTROL_STATE_FILE);
    let tmp_path = config_dir.join(".dev-core-log-controls.tmp");
    let state = CoreLogControlFile {
        sections: controls.clone(),
    };
    let content = serde_json::to_string_pretty(&state)
        .map_err(|e| format!("Failed to serialize core log controls: {}", e))?;
    std::fs::write(&tmp_path, content)
        .map_err(|e| format!("Failed to write core log control temp file: {}", e))?;
    std::fs::rename(&tmp_path, &path)
        .map_err(|e| format!("Failed to finalize core log control file: {}", e))
}

#[cfg(feature = "dev")]
pub fn upsert_core_control(
    config_dir: &Path,
    section: &str,
    mut control: LogControl,
) -> Result<(), String> {
    control.suppress_patterns = normalize_patterns(control.suppress_patterns);
    let mut controls = load_all_core_controls(config_dir);
    if control.muted || !control.suppress_patterns.is_empty() {
        controls.insert(section.to_string(), control);
    } else {
        controls.remove(section);
    }
    save_all_core_controls(config_dir, &controls)
}
```

**Step 3: Add core control exports to `src/logging/mod.rs`**

```rust
#[cfg(feature = "dev")]
pub use control::{load_all_core_controls, upsert_core_control};
```

**Step 4: Add tests to `src/logging/control.rs`**

Add inside the existing `#[cfg(test)] mod tests` block:

```rust
#[test]
fn upsert_core_control_roundtrip_and_clear() {
    let tmp = TempDir::new().unwrap();

    upsert_core_control(
        tmp.path(),
        "runtime",
        LogControl {
            muted: true,
            suppress_patterns: vec![],
        },
    )
    .unwrap();

    let loaded = load_all_core_controls(tmp.path());
    assert_eq!(loaded.len(), 1);
    assert!(loaded["runtime"].muted);

    upsert_core_control(
        tmp.path(),
        "runtime",
        LogControl {
            muted: false,
            suppress_patterns: vec![],
        },
    )
    .unwrap();

    let cleared = load_all_core_controls(tmp.path());
    assert!(cleared.is_empty());
}
```

### Task 8: Add core_log_controls to AppState

**Files:**
- Modify: `src/features/plugin_store/server/types.rs`

**Step 1: Add field to AppState**

```rust
#[cfg(feature = "dev")]
pub(super) core_log_controls: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, crate::logging::LogControl>>>,
```

**Step 2: Initialize in `AppState::new`**

In the `Self { ... }` block, add:

```rust
#[cfg(feature = "dev")]
core_log_controls: std::sync::Arc::new(std::sync::RwLock::new(
    crate::logging::load_all_core_controls(&plugins_dir.parent().unwrap_or(&plugins_dir)),
)),
```

Note: This is a temporary initialization that loads from disk. In the final integration (Phase 3), this will be replaced by the shared handle from `main.rs` passed through `start_ui_server`. For now, loading independently is fine — the two instances will converge when merged.

### Task 9: Create core log control API endpoints

**Files:**
- Create: `src/features/plugin_store/server/dev_core_log_handlers.rs`
- Modify: `src/features/plugin_store/server.rs`

**Step 1: Create `dev_core_log_handlers.rs`**

```rust
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, put},
    Json, Router,
};

use super::helpers::{shared_config_dir_or_response, validate_plugin_id_bad_request};
use super::types::{AppState, UpsertPluginLogControlRequest};

const VALID_SECTIONS: &[&str] = &["runtime", "plugins", "core"];

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/dev/core-log-controls", get(get_core_log_controls))
        .route(
            "/dev/core-log-controls/{section}",
            put(upsert_core_log_control),
        )
}

async fn get_core_log_controls(
    State(state): State<AppState>,
) -> Json<std::collections::HashMap<String, crate::logging::LogControl>> {
    let controls = state
        .core_log_controls
        .read()
        .map(|c| c.clone())
        .unwrap_or_default();
    Json(controls)
}

async fn upsert_core_log_control(
    Path(section): Path<String>,
    State(state): State<AppState>,
    Json(req): Json<UpsertPluginLogControlRequest>,
) -> impl IntoResponse {
    if !VALID_SECTIONS.contains(&section.as_str()) {
        return (
            StatusCode::BAD_REQUEST,
            format!("Invalid section: {}. Valid: {:?}", section, VALID_SECTIONS),
        )
            .into_response();
    }

    let config_dir = match shared_config_dir_or_response("Config dir unavailable") {
        Ok(d) => d,
        Err(e) => return e.into_response(),
    };

    let control = crate::logging::LogControl {
        muted: req.muted,
        suppress_patterns: req.suppress_patterns,
    };

    match crate::logging::upsert_core_control(&config_dir, &section, control.clone()) {
        Ok(()) => {
            if let Ok(mut controls) = state.core_log_controls.write() {
                if control.muted || !control.suppress_patterns.is_empty() {
                    controls.insert(section, control);
                } else {
                    controls.remove(&section);
                }
            }
            (StatusCode::OK, "Updated".to_string()).into_response()
        }
        Err(e) => {
            log::error!("Failed to upsert core log control for {}: {}", section, e);
            (StatusCode::INTERNAL_SERVER_ERROR, e).into_response()
        }
    }
}
```

**Step 2: Register module and routes in `src/features/plugin_store/server.rs`**

Add module declaration (with the other dev modules):

```rust
#[cfg(feature = "dev")]
mod dev_core_log_handlers;
```

Add route merge in `api_router` (with the other dev merges):

```rust
.merge(dev_core_log_handlers::routes())
```

### Task 10: Verify and commit

```bash
cargo check --all-features
git add -A
git commit -m "feat: add core log control persistence and API endpoints"
```

---

## Phase 2C: Frontend UI (Parallel Worktree)

Branch from Phase 1 merge commit. Independent of Phase 2A and 2B.

### Task 11: Create core log section template

**Files:**
- Create: `ui/views/dev/core-log-template.js`

```javascript
import { escapeAttr } from '../../utils/escape-html.js';

const CORE_SECTIONS = [
    { id: 'runtime', name: 'Runtime', description: 'Socket, state, polling' },
    { id: 'plugins', name: 'Plugins', description: 'Daemon lifecycle, loading' },
    { id: 'core', name: 'Core', description: 'Tray, hotkeys, menu, updates' }
];

export function renderCoreLogSection(state) {
    const rows = CORE_SECTIONS.map(section => renderCoreLogRow(state, section)).join('');
    return `
        <section class="dev-section">
            <div class="section-header">
                <h2>Core Logs</h2>
            </div>
            <div class="plugin-list-container">
                <div class="plugin-list table-list">${rows}</div>
            </div>
        </section>
    `;
}

function renderCoreLogRow(state, section) {
    const control = state.coreLogControls[section.id] || {};
    const muted = !!control.muted;
    const patterns = Array.isArray(control.suppress_patterns) ? control.suppress_patterns : [];
    const filterCount = patterns.length;
    const sectionId = escapeAttr(section.id);
    const menuOpen = state.openCoreMenuId === section.id;

    return `
        <div class="plugin-row table-list-row status-linked core-log-row" data-core-section="${sectionId}">
            <div class="plugin-main table-grid">
                <div class="plugin-info table-col">
                    <div class="plugin-copy">
                        <div class="plugin-title-row">
                            <span class="plugin-name">${section.name}</span>
                        </div>
                        <span class="plugin-path">${section.description}</span>
                    </div>
                    ${muted ? '<div class="plugin-status-badges"><span class="status-badge badge-muted">Muted</span></div>' : ''}
                </div>
                <div class="plugin-action-column table-col">
                    ${renderCoreLogMenu(sectionId, section.name, muted, filterCount, menuOpen)}
                </div>
            </div>
        </div>
    `;
}

function renderCoreLogMenu(sectionId, sectionName, muted, filterCount, menuOpen) {
    return `
        <button type="button" class="plugin-menu-trigger" data-action="toggle-core-menu" data-id="${sectionId}" aria-label="Log options for ${sectionName}" aria-expanded="${menuOpen ? 'true' : 'false'}">
            <svg class="plugin-menu-trigger-icon" viewBox="0 0 12 20" fill="currentColor" aria-hidden="true" focusable="false">
                <circle cx="6" cy="3.5" r="1.8"></circle>
                <circle cx="6" cy="10" r="1.8"></circle>
                <circle cx="6" cy="16.5" r="1.8"></circle>
            </svg>
        </button>
        <div class="plugin-context-menu ${menuOpen ? 'open' : ''}">
            <button type="button" class="context-action" data-action="toggle-core-logs" data-id="${sectionId}" aria-label="${muted ? 'Unmute' : 'Mute'} ${sectionName} logs">
                ${muted ? 'Unmute Logs' : 'Mute Logs'}
            </button>
            <button type="button" class="context-action" data-action="edit-core-log-filters" data-id="${sectionId}" aria-label="Edit log filters for ${sectionName}">
                ${filterCount > 0 ? `Edit Filters (${filterCount})` : 'Edit Filters'}
            </button>
        </div>
    `;
}
```

### Task 12: Create core log actions

**Files:**
- Create: `ui/views/dev/core-log-actions.js`

```javascript
import { jsonRequest, readResponseText } from '../../api/client.js';

export function createCoreLogActions({ state, discoveryController, onNeedsRender }) {
    return {
        toggleCoreLogs: id => toggleCoreLogs(state, discoveryController, onNeedsRender, id),
        editCoreLogFilters: id => editCoreLogFilters(state, discoveryController, onNeedsRender, id)
    };
}

async function toggleCoreLogs(state, discoveryController, onNeedsRender, sectionId) {
    const control = state.coreLogControls[sectionId] || {};
    try {
        await saveCoreLogControl(sectionId, {
            muted: !control.muted,
            suppress_patterns: Array.isArray(control.suppress_patterns) ? control.suppress_patterns : []
        });
        await discoveryController.loadCoreLogControls(true);
    } catch (error) {
        state.error = error?.message || 'Failed to toggle core logs';
    }
    onNeedsRender();
}

async function editCoreLogFilters(state, discoveryController, onNeedsRender, sectionId) {
    const control = state.coreLogControls[sectionId] || {};
    const current = Array.isArray(control.suppress_patterns) ? control.suppress_patterns : [];
    const value = window.prompt(
        'Mute log lines containing these comma-separated substrings (leave empty to clear):',
        current.join(', ')
    );
    if (value === null) return;
    try {
        await saveCoreLogControl(sectionId, {
            muted: !!control.muted,
            suppress_patterns: normalizePatternsInput(value)
        });
        await discoveryController.loadCoreLogControls(true);
    } catch (error) {
        state.error = error?.message || 'Failed to update core log filters';
    }
    onNeedsRender();
}

function normalizePatternsInput(raw) {
    if (!raw) return [];
    return raw.split(',').map(v => v.trim()).filter(Boolean);
}

async function saveCoreLogControl(sectionId, control) {
    const response = await fetch(`/api/dev/core-log-controls/${encodeURIComponent(sectionId)}`, {
        ...jsonRequest('PUT', control)
    });
    if (response.ok) return;
    const message = await readResponseText(response);
    throw new Error(message || 'Failed to update core log control');
}
```

### Task 13: Wire into dev page

**Files to modify:**
- `ui/views/dev/template.js`
- `ui/views/dev/index.js`
- `ui/views/dev/discovery-controller.js`
- `ui/views/dev/action-router.js`

**Step 1: Update `template.js`**

Add import:
```javascript
import { renderCoreLogSection } from './core-log-template.js';
```

In `renderDevView`, add `renderCoreLogSection` between plugins and actions:
```javascript
${renderPluginsSection(state, mergedList, pluginRows)}
${renderCoreLogSection(state)}
${renderActionsSection(state, renderBuildResults)}
```

**Step 2: Update `index.js` state**

Add to the `state` object:
```javascript
coreLogControls: {},
openCoreMenuId: null,
```

Add import:
```javascript
import { createCoreLogActions } from './core-log-actions.js';
```

Create core log actions controller (after `actionsController`):
```javascript
const coreLogActions = createCoreLogActions({
    state,
    discoveryController,
    onNeedsRender: updateView
});
```

Pass `coreLogActions` to `routeDevClick` context.

Add `loadCoreLogControls` to the `render()` hydration Promise.all:
```javascript
discoveryController.loadCoreLogControls(true),
```

Add to `onFocus()` Promise.all:
```javascript
discoveryController.loadCoreLogControls(true),
```

**Step 3: Update `discovery-controller.js`**

Add to the returned object:
```javascript
loadCoreLogControls: skip => loadCoreLogControls(ctx, skip),
```

Add function:
```javascript
async function loadCoreLogControls(ctx, skipUpdate = false) {
    const payload = await tryFetchJson('/api/dev/core-log-controls');
    if (payload) ctx.state.coreLogControls = payload;
    maybeRender(ctx, skipUpdate);
}
```

**Step 4: Update `action-router.js`**

Add `coreLogActions` to the destructured parameters of `routeDevClick`.

Add to `dispatchMenuItemAction` (or add new dispatch function):
```javascript
if (action === 'toggle-core-menu' && actionId) {
    event.preventDefault();
    event.stopPropagation();
    state.openCoreMenuId = state.openCoreMenuId === actionId ? null : actionId;
    syncPluginMenuDom();
    return true;
}
if (action === 'toggle-core-logs' && actionId) {
    runMenuAction(event, closePluginMenu, syncPluginMenuDom, () => {
        state.openCoreMenuId = null;
        void coreLogActions.toggleCoreLogs(actionId);
    });
    return true;
}
if (action === 'edit-core-log-filters' && actionId) {
    runMenuAction(event, closePluginMenu, syncPluginMenuDom, () => {
        state.openCoreMenuId = null;
        void coreLogActions.editCoreLogFilters(actionId);
    });
    return true;
}
```

### Task 14: Commit

```bash
git add -A
git commit -m "feat: add core logs section to dev page UI"
```

---

## Phase 3: Integration

After merging all Phase 2 worktrees into the branch with Phase 1.

### Task 15: Wire shared CoreControlsHandle through AppState

In Phase 2A, `main.rs` creates the `CoreControlsHandle`. In Phase 2B, `AppState` creates its own independent copy. After merge, unify them so the API handler and the FilterableLogger share the same `Arc<RwLock<...>>`.

**Files:**
- Modify: `src/main.rs` — pass `core_log_controls` to `start_ui_server`
- Modify: `src/features/plugin_store/server.rs` — accept handle in `start_ui_server`
- Modify: `src/features/plugin_store/server/types.rs` — `AppState::new` accepts handle parameter

**Step 1: Update `start_ui_server` signature**

```rust
pub(crate) async fn start_ui_server(
    plugin_manager: Arc<Mutex<PluginManager>>,
    daemon: &Daemon,
    #[cfg(feature = "dev")]
    core_log_controls: crate::logging::CoreControlsHandle,
) -> Result<u16> {
```

**Step 2: Update `AppState::new` to accept the handle**

Replace the independent `Arc::new(RwLock::new(...))` with the passed-in handle.

**Step 3: Thread through main.rs**

Pass `core_log_controls` from `InitResult` into `start_ui_server`.

### Task 16: Verify everything

```bash
cargo check --all-features
cargo check
cargo clippy --all-targets --all-features -- -D warnings
```

### Task 17: Final commit

```bash
git add -A
git commit -m "feat: wire shared core log controls through AppState"
```

---

## CSS Note

The core log rows reuse existing `.plugin-row`, `.table-list-row`, `.plugin-main`, `.table-grid`, `.plugin-info`, `.plugin-action-column`, `.plugin-context-menu` classes. A `.badge-muted` class may need adding to `dev-plugin-list.css` if it doesn't already exist — check existing badge styles first. The `.core-log-row` class is available for any core-specific overrides but should not be needed initially.

---

## Validation Checklist

- [ ] `cargo check` passes (no dev feature)
- [ ] `cargo check --all-features` passes
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes
- [ ] Existing plugin log controls still work (mute, suppress patterns, daemon restart)
- [ ] Core log sections appear on dev page below plugin list
- [ ] Muting a core section suppresses those log records
- [ ] Suppress patterns filter core log records by message content
- [ ] Controls persist across restart (saved to `dev-core-log-controls.json`)
- [ ] Non-dev builds use plain env_logger with zero overhead
