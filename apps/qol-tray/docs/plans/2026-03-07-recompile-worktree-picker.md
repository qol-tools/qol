# Recompile Worktree Picker Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a worktree picker to the dev UI's recompile card so the user can select and persist a default git worktree to build from.

**Architecture:** New `GET /api/dev/worktrees` endpoint scans `.worktrees/**` for valid source trees. The `POST /api/dev/recompile-self` endpoint gains an optional `worktree_path` body field threaded down to the build function. The frontend `ActionsSection` gains a `RecompileCard` with a split button (recompile from default) + chevron (opens a searchable picker panel above the card) with the default persisted in `localStorage`.

**Tech Stack:** Rust/Axum (backend), Preact + htm template literals (frontend), plain CSS custom properties (styles). Tests use `tempfile::TempDir` for filesystem fixtures.

---

**Worktree:** All work happens in `.worktrees/feat/recompile-worktree-picker/`. Commands assume CWD is that path unless stated.

---

### Task 1: Extend build function to accept optional repo root

**Files:**
- Modify: `src/dev/build/cargo_build/self_build.rs`
- Modify: `src/dev/build/cargo_build.rs`

The inner build function and its public re-export each gain an `Option<&Path>` parameter for the repo root. When `None`, the existing compile-time constant is used.

**Step 1: Update the inner function signature in `self_build.rs`**

Change the function at line 14 from:
```rust
pub(super) fn build_qol_tray_self_with_progress<F>(mut on_progress: F) -> BuildResult
where
    F: FnMut(u8, String),
{
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
```
to:
```rust
pub(super) fn build_qol_tray_self_with_progress<F>(
    repo_root: Option<&Path>,
    mut on_progress: F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    let repo_root = repo_root
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
```

**Step 2: Update the public wrapper in `cargo_build.rs`**

Change line 42–47:
```rust
pub fn build_qol_tray_self_with_progress<F>(repo_root: Option<&Path>, on_progress: F) -> BuildResult
where
    F: FnMut(u8, String),
{
    self_build::build_qol_tray_self_with_progress(repo_root, on_progress)
}
```

**Step 3: Fix the call site in `src/features/plugin_store/server/dev_services/recompile/start.rs`**

The `spawn_self_recompile` function currently calls `dev::build_qol_tray_self_with_progress` with no path. Update to pass `None` so it compiles:
```rust
dev::build_qol_tray_self_with_progress(None, |percent, phase| {
    events.send(DaemonEvent::SelfRecompileProgress { percent, phase });
})
```

**Step 4: Verify it compiles**

```bash
cargo check --features dev 2>&1 | grep -E "^error"
```
Expected: no error lines.

**Step 5: Commit**

```bash
git add src/dev/build/cargo_build/self_build.rs \
        src/dev/build/cargo_build.rs \
        src/features/plugin_store/server/dev_services/recompile/start.rs
git commit -m "refactor: add optional repo_root to build_qol_tray_self_with_progress"
```

---

### Task 2: Wire optional worktree path through the recompile request

**Files:**
- Modify: `src/features/plugin_store/server/types.rs`
- Modify: `src/features/plugin_store/server/dev_services/recompile.rs`
- Modify: `src/features/plugin_store/server/dev_services/recompile/start.rs`
- Modify: `src/features/plugin_store/server/dev_services/mod.rs`
- Modify: `src/features/plugin_store/server/dev_handlers.rs`

**Step 1: Add `RecompileSelfRequest` to `types.rs`**

After the existing `SetPluginCpuMonitoringRequest` block, add:
```rust
#[cfg(feature = "dev")]
#[derive(Debug, Clone, Deserialize, Default)]
pub(super) struct RecompileSelfRequest {
    pub(super) worktree_path: Option<String>,
}
```

**Step 2: Add `worktree_path` to `SelfRecompileTask` in `recompile.rs`**

Add the field and update `from_state`:
```rust
struct SelfRecompileTask {
    events: Arc<crate::daemon::EventBus>,
    plugin_manager: Arc<Mutex<crate::plugins::PluginManager>>,
    runtime: Arc<DevRuntimeService>,
    restart: Arc<dyn RestartPort>,
    worktree_path: Option<PathBuf>,
}

impl SelfRecompileTask {
    fn from_state(state: &AppState, worktree_path: Option<PathBuf>) -> Self {
        Self {
            events: state.daemon.events.clone(),
            plugin_manager: state.plugin_manager.clone(),
            runtime: state.runtime.clone(),
            restart: state.restart.clone(),
            worktree_path,
        }
    }
}
```

Also add `use std::path::PathBuf;` to the imports in `recompile.rs`.

Update `queue_self_recompile` in `recompile.rs`:
```rust
pub(super) fn queue_self_recompile(
    state: &AppState,
    worktree_path: Option<PathBuf>,
) -> Result<(), &'static str> {
    start::queue_self_recompile(state, worktree_path)
}
```

**Step 3: Thread the path through `start.rs`**

Update `queue_self_recompile`:
```rust
pub(super) fn queue_self_recompile(
    state: &AppState,
    worktree_path: Option<PathBuf>,
) -> Result<(), &'static str> {
    if !state.runtime.try_start_self_recompile() {
        return Err("Self recompile already in progress");
    }
    log::info!("Developer self recompile requested");
    tokio::spawn(run_self_recompile(SelfRecompileTask::from_state(state, worktree_path)));
    Ok(())
}
```

Update `run_self_recompile` to forward the path:
```rust
async fn run_self_recompile(task: SelfRecompileTask) {
    let _guard = RecompileGuard {
        runtime: Arc::clone(&task.runtime),
    };
    let result = spawn_self_recompile(Arc::clone(&task.events), task.worktree_path).await;
    result::handle_recompile_result(task, result);
}

async fn spawn_self_recompile(
    events: Arc<EventBus>,
    worktree_path: Option<PathBuf>,
) -> RecompileResult {
    tokio::task::spawn_blocking(move || {
        dev::build_qol_tray_self_with_progress(worktree_path.as_deref(), |percent, phase| {
            events.send(DaemonEvent::SelfRecompileProgress { percent, phase });
        })
    })
    .await
}
```

**Step 4: Update `dev_services/mod.rs`**

Change the `queue_self_recompile` signature:
```rust
pub(super) fn queue_self_recompile(
    state: &AppState,
    worktree_path: Option<std::path::PathBuf>,
) -> Result<(), &'static str> {
    recompile::queue_self_recompile(state, worktree_path)
}
```

**Step 5: Update the handler in `dev_handlers.rs`**

Add `Json` and the new request type to imports:
```rust
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::path::PathBuf;
```

Replace the `recompile_self` handler:
```rust
pub(super) async fn recompile_self(
    State(state): State<AppState>,
    body: Option<Json<super::types::RecompileSelfRequest>>,
) -> impl IntoResponse {
    let worktree_path = body
        .and_then(|Json(req)| req.worktree_path)
        .map(PathBuf::from);
    if let Some(ref path) = worktree_path {
        if !path.join("Cargo.toml").is_file() {
            return (StatusCode::BAD_REQUEST, "No Cargo.toml at worktree path").into_response();
        }
    }
    if let Err(message) = dev_services::queue_self_recompile(&state, worktree_path) {
        return (StatusCode::CONFLICT, message).into_response();
    }
    StatusCode::ACCEPTED.into_response()
}
```

**Step 6: Verify it compiles**

```bash
cargo check --features dev 2>&1 | grep -E "^error"
```
Expected: no error lines.

**Step 7: Write unit test for `RecompileSelfRequest` deserialization**

Add to the bottom of `types.rs` (inside `#[cfg(test)]`):
```rust
#[cfg(test)]
mod tests {
    use super::RecompileSelfRequest;

    #[test]
    fn recompile_request_path_is_optional() {
        let cases = [
            (r#"{}"#, None),
            (r#"{"worktree_path":null}"#, None),
            (r#"{"worktree_path":"/a/b/c"}"#, Some("/a/b/c")),
        ];
        for (input, expected_path) in cases {
            let req: RecompileSelfRequest = serde_json::from_str(input)
                .unwrap_or_else(|e| panic!("failed to parse {:?}: {}", input, e));
            assert_eq!(
                req.worktree_path.as_deref(),
                expected_path,
                "input: {}",
                input
            );
        }
    }
}
```

**Step 8: Run the test**

```bash
cargo test --features dev recompile_request_path_is_optional 2>&1 | tail -5
```
Expected: `test ... ok`

**Step 9: Commit**

```bash
git add src/features/plugin_store/server/types.rs \
        src/features/plugin_store/server/dev_services/recompile.rs \
        src/features/plugin_store/server/dev_services/recompile/start.rs \
        src/features/plugin_store/server/dev_services/mod.rs \
        src/features/plugin_store/server/dev_handlers.rs
git commit -m "feat: accept optional worktree_path in recompile-self endpoint"
```

---

### Task 3: Add `GET /api/dev/worktrees` endpoint

**Files:**
- Create: `src/features/plugin_store/server/dev_services/worktrees.rs`
- Modify: `src/features/plugin_store/server/dev_services/mod.rs`
- Modify: `src/features/plugin_store/server/types.rs`
- Modify: `src/features/plugin_store/server/dev_handlers.rs`

**Step 1: Add `WorktreeInfo` to `types.rs`**

After `RecompileSelfRequest`:
```rust
#[cfg(feature = "dev")]
#[derive(Debug, Clone, Serialize)]
pub(super) struct WorktreeInfo {
    pub(super) branch: String,
    pub(super) path: String,
}
```

**Step 2: Write the failing test first in a new `worktrees.rs`**

Create `src/features/plugin_store/server/dev_services/worktrees.rs`:
```rust
use std::path::{Path, PathBuf};

use super::super::types::WorktreeInfo;

pub(super) fn scan(manifest_dir: &Path) -> Vec<WorktreeInfo> {
    collect(&manifest_dir.join(".worktrees"), &manifest_dir.join(".worktrees"), 0)
}

fn collect(root: &Path, dir: &Path, depth: u8) -> Vec<WorktreeInfo> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut results = vec![];
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.join("Cargo.toml").is_file() {
            let branch = path
                .strip_prefix(root)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            results.push(WorktreeInfo {
                branch,
                path: path.to_string_lossy().into_owned(),
            });
        } else if depth < 1 {
            results.extend(collect(root, &path, depth + 1));
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn scan_finds_two_level_worktrees() {
        let tmp = TempDir::new().unwrap();
        let wt = tmp.path().join(".worktrees").join("feat").join("my-feature");
        fs::create_dir_all(&wt).unwrap();
        fs::write(wt.join("Cargo.toml"), "[package]").unwrap();

        let result = scan(tmp.path());
        assert_eq!(result.len(), 1, "should find one worktree");
        assert_eq!(result[0].branch, "feat/my-feature");
        assert_eq!(result[0].path, wt.to_string_lossy().as_ref());
    }

    #[test]
    fn scan_ignores_dirs_without_cargo_toml() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".worktrees").join("feat").join("no-cargo")).unwrap();

        let result = scan(tmp.path());
        assert!(result.is_empty(), "dirs without Cargo.toml must not appear");
    }

    #[test]
    fn scan_returns_empty_when_no_worktrees_dir() {
        let tmp = TempDir::new().unwrap();
        let result = scan(tmp.path());
        assert!(result.is_empty(), "missing .worktrees dir returns empty");
    }

    #[test]
    fn scan_finds_multiple_worktrees() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join(".worktrees");
        for branch in ["feat/foo", "feat/bar", "refactor/baz"] {
            let wt = root.join(branch);
            fs::create_dir_all(&wt).unwrap();
            fs::write(wt.join("Cargo.toml"), "[package]").unwrap();
        }

        let mut result = scan(tmp.path());
        result.sort_by(|a, b| a.branch.cmp(&b.branch));
        let branches: Vec<&str> = result.iter().map(|w| w.branch.as_str()).collect();
        assert_eq!(branches, ["feat/bar", "feat/foo", "refactor/baz"]);
    }
}
```

**Step 3: Run the tests to verify they pass (they should, since implementation and tests are written together)**

```bash
cargo test --features dev worktrees:: 2>&1 | tail -10
```
Expected: all 4 tests `ok`.

**Step 4: Expose `list_worktrees` from `dev_services/mod.rs`**

Add the module and public function:
```rust
mod worktrees;

pub(super) fn list_worktrees() -> Vec<super::types::WorktreeInfo> {
    worktrees::scan(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
}
```

**Step 5: Add the handler and route in `dev_handlers.rs`**

In the `routes()` function, add the new GET route:
```rust
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/dev/reload", post(reload_plugins))
        .route("/dev/reload/{plugin_id}", post(reload_single_plugin))
        .route("/dev/recompile-self", post(recompile_self))
        .route("/dev/worktrees", get(list_worktrees))
}
```

Add the handler function:
```rust
async fn list_worktrees() -> impl IntoResponse {
    Json(dev_services::list_worktrees())
}
```

**Step 6: Verify compilation**

```bash
cargo check --features dev 2>&1 | grep -E "^error"
```
Expected: no error lines.

**Step 7: Commit**

```bash
git add src/features/plugin_store/server/dev_services/worktrees.rs \
        src/features/plugin_store/server/dev_services/mod.rs \
        src/features/plugin_store/server/types.rs \
        src/features/plugin_store/server/dev_handlers.rs
git commit -m "feat: add GET /api/dev/worktrees endpoint"
```

---

### Task 4: Add recompile state and worktree state to the dev controller

**Files:**
- Modify: `ui/views/dev/use-controller.js`

This task wires the backend into the frontend controller: fetch worktrees on mount, track recompile progress via SSE, and expose a `triggerRecompile` callback and `setDefaultWorktree`.

**Step 1: Add recompile and worktree fields to `createInitialState`**

Add to the returned object in `createInitialState()`:
```js
recompile: { active: false, percent: 0, phase: '', error: null, done: false, clearTimer: null },
worktrees: [],
defaultWorktree: loadDefaultWorktree(),
```

Add the `loadDefaultWorktree` helper before `createInitialState`:
```js
const RECOMPILE_WT_KEY = 'dev.recompile.defaultWorktree';

function loadDefaultWorktree() {
    try { return localStorage.getItem(RECOMPILE_WT_KEY) || null; } catch { return null; }
}

function saveDefaultWorktree(path) {
    try {
        if (path) { localStorage.setItem(RECOMPILE_WT_KEY, path); }
        else { localStorage.removeItem(RECOMPILE_WT_KEY); }
    } catch {}
}
```

**Step 2: Add SSE handlers for recompile events in `handleSSEEvent`**

Insert before the `cpuController` line:
```js
if (event.type === 'self_recompile_progress') {
    state.recompile.active = true;
    state.recompile.percent = event.percent || 0;
    state.recompile.phase = event.phase || '';
    state.recompile.error = null;
    bump();
    return;
}
if (event.type === 'self_recompile_complete') {
    if (state.recompile.clearTimer) clearTimeout(state.recompile.clearTimer);
    state.recompile.active = false;
    state.recompile.done = true;
    bump();
    state.recompile.clearTimer = setTimeout(() => {
        state.recompile.done = false;
        bump();
    }, 2000);
    return;
}
if (event.type === 'self_recompile_failed') {
    state.recompile.active = false;
    state.recompile.error = event.message || 'Recompile failed';
    bump();
    return;
}
```

**Step 3: Add worktree fetch and `triggerRecompile` to `useHydration`**

In the `useHydration` function, add a worktrees fetch inside the `useEffect`:
```js
fetch('/api/dev/worktrees')
    .then(r => r.ok ? r.json() : [])
    .then(list => { state.worktrees = Array.isArray(list) ? list : []; })
    .catch(() => {});
```

(Add this in the existing `useEffect` call, alongside the other `void Promise.all(...)` block.)

**Step 4: Add `triggerRecompile` and `setDefaultWorktree` to `buildActionCallbacks`**

In `buildLinkCallbacks`, add:
```js
triggerRecompile: () => void triggerRecompile(state, bump),
setDefaultWorktree: (path) => { saveDefaultWorktree(path); state.defaultWorktree = path; bump(); },
```

Add the standalone `triggerRecompile` function outside the hook (module-level):
```js
async function triggerRecompile(state, bump) {
    if (state.recompile.active) return;
    state.recompile = { active: true, percent: 0, phase: 'Preparing build', error: null, done: false, clearTimer: null };
    bump();
    try {
        const opts = { method: 'POST' };
        if (state.defaultWorktree) {
            opts.headers = { 'Content-Type': 'application/json' };
            opts.body = JSON.stringify({ worktree_path: state.defaultWorktree });
        }
        const res = await fetch('/api/dev/recompile-self', opts);
        if (!res.ok) {
            const text = await res.text().catch(() => '');
            state.recompile.active = false;
            state.recompile.error = text || `Recompile failed (${res.status})`;
            bump();
        }
    } catch (err) {
        state.recompile.active = false;
        state.recompile.error = err.message || 'Recompile failed';
        bump();
    }
}
```

**Step 5: Commit**

```bash
git add ui/views/dev/use-controller.js
git commit -m "feat: add recompile state and worktree picker state to dev controller"
```

---

### Task 5: Add `RecompileCard` component with split button and picker panel

**Files:**
- Modify: `ui/views/dev/components/ActionsSection.js`
- Modify: `ui/styles/dev-layout.css`

**Step 1: Add the `RecompileCard` component to `ActionsSection.js`**

Add the import for hooks at the top of `ActionsSection.js`:
```js
import { useState } from 'preact/hooks';
```

Add the `RecompileCard` function before `ActionsSection`:

```js
function RecompileCard({ recompile, worktrees, defaultWorktree, setDefaultWorktree, triggerRecompile }) {
    const [pickerOpen, setPickerOpen] = useState(false);
    const [query, setQuery] = useState('');
    const [highlightIdx, setHighlightIdx] = useState(0);

    const options = [{ branch: 'main', path: null }, ...worktrees];
    const filtered = query
        ? options.filter(o => o.branch.toLowerCase().includes(query.toLowerCase()))
        : options;

    const currentBranch = defaultWorktree
        ? (worktrees.find(w => w.path === defaultWorktree)?.branch ?? 'main')
        : 'main';

    function openPicker(e) {
        e.stopPropagation();
        setPickerOpen(v => !v);
        setQuery('');
        setHighlightIdx(0);
    }

    function selectOption(opt) {
        setDefaultWorktree(opt.path);
        setPickerOpen(false);
        setQuery('');
    }

    function handlePickerKey(e) {
        if (e.key === 'ArrowDown') { e.preventDefault(); setHighlightIdx(i => Math.min(i + 1, filtered.length - 1)); return; }
        if (e.key === 'ArrowUp') { e.preventDefault(); setHighlightIdx(i => Math.max(i - 1, 0)); return; }
        if (e.key === 'Enter') { e.preventDefault(); if (filtered[highlightIdx]) selectOption(filtered[highlightIdx]); return; }
        if (e.key === 'Escape') { setPickerOpen(false); }
    }

    const isActive = recompile.active;
    const btnLabel = isActive
        ? (recompile.percent > 0 ? `${recompile.percent}%` : 'Recompiling...')
        : recompile.done ? 'Done'
        : currentBranch;

    return html`
        <div class="dev-card recompile-card">
            <button class=${'refresh-btn ' + (isActive ? 'spinning' : '')} tabindex="-1" aria-hidden="true"></button>
            <div class="dev-card-content">
                <h3>Recompile QoL Tray</h3>
                ${recompile.error
                    ? html`<span class="error-msg">${recompile.error}</span>`
                    : html`<p>${isActive ? (recompile.phase || 'Building...') : 'Build and restart the daemon.'}</p>`}
            </div>
            <div class="recompile-action-wrap">
                ${pickerOpen && html`
                    <div class="recompile-picker">
                        <input
                            class="recompile-search"
                            type="text"
                            placeholder="Filter branches..."
                            value=${query}
                            onInput=${e => { setQuery(e.target.value); setHighlightIdx(0); }}
                            onKeyDown=${handlePickerKey}
                            autoFocus
                        />
                        <ul class="recompile-list" role="listbox">
                            ${filtered.map((opt, i) => html`
                                <li key=${opt.branch}
                                    class=${'recompile-option'
                                        + (i === highlightIdx ? ' is-highlighted' : '')
                                        + (opt.path === defaultWorktree || (!opt.path && !defaultWorktree) ? ' is-selected' : '')}
                                    role="option"
                                    onClick=${() => selectOption(opt)}
                                    onMouseEnter=${() => setHighlightIdx(i)}
                                >${opt.branch}</li>
                            `)}
                            ${filtered.length === 0 && html`<li class="recompile-option is-empty">No matches</li>`}
                        </ul>
                    </div>
                `}
                <div class="recompile-btn-row">
                    <button
                        class="btn btn-primary recompile-btn"
                        disabled=${isActive}
                        onClick=${() => { if (!isActive) triggerRecompile(); }}
                    >↺ ${btnLabel}</button>
                    <button
                        class="btn btn-ghost recompile-chevron"
                        onClick=${openPicker}
                        aria-label="Pick recompile source"
                    >▾</button>
                </div>
            </div>
        </div>
    `;
}
```

**Step 2: Add `RecompileCard` to the `ActionsSection` render**

In `ActionsSection`, add after `MockCard`:
```js
<${RecompileCard}
    recompile=${ctrl.recompile}
    worktrees=${ctrl.worktrees}
    defaultWorktree=${ctrl.defaultWorktree}
    setDefaultWorktree=${ctrl.setDefaultWorktree}
    triggerRecompile=${ctrl.triggerRecompile}
/>
```

**Step 3: Add CSS to `dev-layout.css`**

Append to the end of `dev-layout.css`:
```css
.recompile-card {
    cursor: default;
    align-items: flex-start;
}

.recompile-card:hover {
    transform: none;
}

.recompile-action-wrap {
    position: relative;
    flex-shrink: 0;
}

.recompile-btn-row {
    display: flex;
    gap: var(--space-1);
}

.recompile-btn {
    min-width: 140px;
    justify-content: flex-start;
    font-family: var(--font-mono, monospace);
    font-size: var(--fs-sm);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.recompile-chevron {
    padding: 0 var(--space-3);
    font-size: var(--fs-caption);
}

.recompile-picker {
    position: absolute;
    bottom: calc(100% + var(--space-2));
    right: 0;
    width: 260px;
    background: var(--bg-surface);
    border: var(--border-w-1) solid var(--border-default);
    border-radius: var(--radius-lg);
    box-shadow: 0 8px 24px var(--layer-ink-28);
    z-index: 20;
    overflow: hidden;
}

.recompile-search {
    width: 100%;
    padding: var(--space-3) var(--space-4);
    background: transparent;
    border: none;
    border-bottom: var(--border-w-1) solid var(--border-subtle);
    color: var(--text-secondary);
    font-size: var(--fs-md);
    font-family: inherit;
    outline: none;
    box-sizing: border-box;
}

.recompile-search:focus {
    color: var(--text-primary);
    border-bottom-color: var(--accent);
}

.recompile-list {
    list-style: none;
    margin: 0;
    padding: var(--space-1) 0;
    max-height: 200px;
    overflow-y: auto;
}

.recompile-option {
    padding: var(--space-2) var(--space-4);
    font-size: var(--fs-md);
    font-family: var(--font-mono, monospace);
    color: var(--text-secondary);
    cursor: pointer;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.recompile-option.is-highlighted {
    background: var(--bg-hover);
    color: var(--text-primary);
}

.recompile-option.is-selected {
    color: var(--accent);
}

.recompile-option.is-selected.is-highlighted {
    background: var(--state-info-bg-faint);
}

.recompile-option.is-empty {
    color: var(--text-faint);
    cursor: default;
    font-family: inherit;
}
```

**Step 4: Commit**

```bash
git add ui/views/dev/components/ActionsSection.js ui/styles/dev-layout.css
git commit -m "feat: add RecompileCard with worktree picker to dev ActionsSection"
```

---

### Task 6: Verify end-to-end in the dev UI

**Step 1: Start the dev build**

From the repo root (not the worktree):
```bash
make dev TREE=feat/recompile-worktree-picker
```

**Step 2: Open the dev view**

Navigate to the Dev page. The Actions section should show three cards: Reload All Plugins, Test mock flows, and Recompile QoL Tray.

**Step 3: Verify worktree picker**

- The card shows `↺ main` (default)
- Clicking `▾` opens the picker panel above the card
- The search input is focused
- Typing `feat` filters to only feat/* branches
- Arrow keys move the highlight
- Enter selects and closes the picker
- The button label updates to the selected branch (e.g. `↺ feat/recompile-worktree-picker`)
- Refreshing the page preserves the selection

**Step 4: Verify recompile**

- Click `↺ feat/recompile-worktree-picker`
- The button shows `↺ Recompiling...` then a percent counter
- The sidebar footer also shows the recompile progress
- On success the button briefly shows `↺ Done` then reverts to the branch name
- The daemon restarts from the selected worktree

**Step 5: Verify invalid path protection**

Manually POST to the endpoint with a bogus path:
```bash
curl -s -o /dev/null -w "%{http_code}" -X POST http://localhost:42700/api/dev/recompile-self \
  -H "Content-Type: application/json" \
  -d '{"worktree_path":"/tmp/not-a-cargo-project"}'
```
Expected: `400`
