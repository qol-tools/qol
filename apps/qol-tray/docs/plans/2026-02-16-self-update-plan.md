# Self-Update UI Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add inline self-update controls to the sidebar so users can check for and install updates from the web UI.

**Architecture:** Two new API endpoints (`GET /check-update`, `POST /self-update`) thin-wrap existing `updates` module. Sidebar component gains update state rendering with morph animation from download button to spinner.

**Tech Stack:** Rust/Axum (backend), vanilla JS + CSS (frontend)

---

### Task 1: Add `GET /check-update` endpoint

**Files:**
- Modify: `qol-tray: src/features/plugin_store/server.rs:176` (add route)
- Modify: `qol-tray: src/features/plugin_store/server.rs:640` (add handler near `get_version`)

**Step 1: Add the handler function**

After `get_version` at line 642, add:

```rust
async fn check_update() -> Json<serde_json::Value> {
    let available = crate::updates::check_for_updates().await.unwrap_or(false);
    let latest = crate::updates::latest_version().map(String::from);
    Json(serde_json::json!({ "available": available, "latest": latest }))
}
```

**Step 2: Register the route**

At line 176, after `.route("/version", get(get_version))`, add:

```rust
.route("/check-update", get(check_update))
```

**Step 3: Commit**

```
feat: add check-update API endpoint
```

---

### Task 2: Add `POST /self-update` endpoint

**Files:**
- Modify: `qol-tray: src/features/plugin_store/server.rs` (same file, add route + handler)

**Step 1: Add the handler function**

After `check_update`, add:

```rust
async fn self_update() -> impl IntoResponse {
    tokio::spawn(async {
        if let Err(e) = crate::updates::download_and_install().await {
            log::error!("Self-update failed: {}", e);
        }
    });
    StatusCode::ACCEPTED
}
```

**Step 2: Register the route**

After the check-update route, add:

```rust
.route("/self-update", post(self_update))
```

**Step 3: Commit**

```
feat: add self-update API endpoint
```

---

### Task 3: Update sidebar to render update controls

**Files:**
- Modify: `qol-tray: ui/components/sidebar.js:9-19`

**Step 1: Update render signature and template**

Replace the entire `render` function and add a `renderUpdateControl` helper:

```javascript
export function render(activeViewId, viewOrder = ['plugins', 'store', 'hotkeys'], version = null, updateState = null) {
    const items = viewOrder.map(id => `
        <div class="sidebar-item ${id === activeViewId ? 'active' : ''}" data-view="${id}">
            ${LABELS[id] || id}
        </div>
    `).join('');

    const versionHtml = version ? `
        <div class="sidebar-version">
            <span class="version-label">v${version}</span>
            ${renderUpdateControl(updateState)}
        </div>
    ` : '';

    return `<div class="sidebar-nav">${items}</div>${versionHtml}`;
}

function renderUpdateControl(state) {
    if (!state || state.status === 'idle') {
        return '';
    }
    if (state.status === 'checking' || state.status === 'downloading') {
        return '<button class="refresh-btn spinning update-btn" disabled></button>';
    }
    if (state.status === 'available') {
        return `<button class="refresh-btn update-btn update-download" data-action="self-update"
                    title="Update (${state.latest})">&#x2B07;</button>
                <span class="update-text">Update (${state.latest})</span>`;
    }
    return `<button class="refresh-btn update-btn" data-action="check-update"
                title="Check for updates">&#x21BB;</button>
            <span class="update-text">Check for updates</span>`;
}
```

States: `idle` (no version yet), `checking`, `up-to-date`, `available`, `downloading`, `error`.

Note: All dynamic values (`state.latest`) come from our own server API responses, not user input. The sidebar is rendered in a local Electron/webview context with no external content.

**Step 2: Commit**

```
feat: render update controls in sidebar
```

---

### Task 4: Add sidebar update CSS

**Files:**
- Modify: `qol-tray: ui/styles.css:119-123` (expand `.sidebar-version`)

**Step 1: Replace `.sidebar-version` block**

Replace lines 119-123 with:

```css
.sidebar-version {
    padding: 0.75rem 1.5rem;
    color: var(--text-disabled);
    font-size: 0.8rem;
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
}

.sidebar-version .update-btn {
    width: 1.5rem;
    height: 1.5rem;
    font-size: 0.7rem;
    border-width: 1.5px;
    transition: color 0.3s, border-color 0.3s, background 0.3s;
}

.sidebar-version .update-download {
    color: var(--accent);
    border-color: var(--accent);
}

.sidebar-version .update-download:hover:not(:disabled) {
    background: var(--accent);
    color: var(--bg-base);
}

.sidebar-version .update-text {
    color: var(--text-muted);
}
```

**Step 2: Commit**

```
feat: add sidebar update button styles with morph transition
```

---

### Task 5: Wire up update state in main.js

**Files:**
- Modify: `qol-tray: ui/main.js:22` (add state variable)
- Modify: `qol-tray: ui/main.js:24-51` (init + updateSidebar)
- Modify: `qol-tray: ui/main.js:104-112` (add click handler)

**Step 1: Add update state**

After `let appVersion = null;` (line 22), add:

```javascript
let updateState = { status: 'checking' };
```

**Step 2: Add check and update functions**

After the `handleSidebarClick` function, add:

```javascript
async function checkForUpdate() {
    updateState = { status: 'checking' };
    updateSidebar();
    try {
        const res = await fetch('/api/check-update');
        if (!res.ok) throw new Error();
        const data = await res.json();
        updateState = data.available
            ? { status: 'available', latest: data.latest }
            : { status: 'up-to-date' };
    } catch {
        updateState = { status: 'error' };
    }
    updateSidebar();
}

async function triggerSelfUpdate() {
    updateState = { status: 'downloading' };
    updateSidebar();
    try {
        await fetch('/api/self-update', { method: 'POST' });
    } catch {
        updateState = { status: 'error' };
        updateSidebar();
    }
}
```

**Step 3: Call checkForUpdate on init**

In `init()`, after `switchView('plugins');` (line 43), add:

```javascript
checkForUpdate();
```

**Step 4: Update `updateSidebar` to pass state**

Change the renderSidebar call in `updateSidebar()` (line 51) to:

```javascript
sidebarEl.innerHTML = renderSidebar(activeViewId, VIEW_ORDER, appVersion, updateState);
```

Note: This follows the existing pattern used throughout the codebase (e.g., same function, plugins.js, store.js, dev.js all use innerHTML assignment for rendering). All content is server-sourced from our own API.

**Step 5: Handle update button clicks**

In `handleSidebarClick`, before the existing `const item = e.target.closest('.sidebar-item');`, add:

```javascript
const updateBtn = e.target.closest('[data-action]');
if (updateBtn) {
    const action = updateBtn.dataset.action;
    if (action === 'check-update') checkForUpdate();
    if (action === 'self-update') triggerSelfUpdate();
    return;
}
```

**Step 6: Commit**

```
feat: wire up self-update UI state and interactions
```

---

### Task 6: Manual testing

Verify all 5 states work:
1. On load: spinner shows briefly, then resolves to "Check for updates" or "Update (x.y.z)"
2. Click the reload button: spinner, then result
3. If update available: blue download button shows with version
4. Click download: morphs to spinner (pkexec dialog appears on Linux)
5. If network error: falls back to "Check for updates" (retry)
