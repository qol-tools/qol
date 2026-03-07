# Recompile Worktree Picker

## Goal

Add a worktree picker to the dev UI's recompile card so that the user can select which git worktree to compile from. The selection persists as a default, so recompiling from a feature branch requires no interaction beyond the initial pick.

## UI

### RecompileCard layout

```
[ ↺ main                           ▼ ]
```

A single row: recompile button (left, full width) + persistent chevron (right). The button label shows the current default branch name — `main`, `feat/recompile-worktree-picker`, etc.

Clicking the **recompile button** fires the build immediately from the stored default. No popup, no confirmation.

Clicking the **chevron** opens a picker panel **above** the card:

```
┌─────────────────────────────┐
│ [ search...                ]│
│─────────────────────────────│
│ ● main                      │
│   feat/plugins-search-header│
│   feat/recompile-worktree-  │
│   picker                    │
└─────────────────────────────┘
[ ↺ main                       ▼ ]
```

- Search input is auto-focused on open
- Typing filters the list by branch name substring
- Arrow keys move the highlighted row, Enter selects, Esc closes
- Selecting a row sets it as the new default and closes the panel
- The current default is marked (bullet or highlight)

The picker renders above the card so it is never clipped at the bottom of the view.

### Recompile state

The recompile button reflects the in-progress/error/done state:

| State  | Button appearance                   |
|--------|-------------------------------------|
| idle   | `↺ main`                            |
| active | spinner + `Recompiling...`          |
| done   | `Done` (clears after ~2s)           |
| error  | error text (short, truncated)       |

## Backend

### New endpoint: `GET /api/dev/worktrees`

Scans `CARGO_MANIFEST_DIR/.worktrees/` recursively for any direct-child-of-two-levels directories that contain a `Cargo.toml`. Returns them as a JSON array.

```json
[
  { "branch": "feat/plugins-search-header",     "path": "/abs/path/to/worktree" },
  { "branch": "feat/recompile-worktree-picker",  "path": "/abs/path/to/worktree" }
]
```

`branch` is the path relative to `.worktrees/` (e.g. the directory `feat/recompile-worktree-picker` maps to branch name `feat/recompile-worktree-picker`).

If `.worktrees/` does not exist (e.g. running from inside a worktree), returns `[]`.

### Modified endpoint: `POST /api/dev/recompile-self`

Accepts an optional JSON body:

```json
{ "worktree_path": "/abs/path/to/worktree" }
```

If `worktree_path` is absent or null, uses `CARGO_MANIFEST_DIR` as the repo root (existing behavior). If provided, validates that the path contains a `Cargo.toml` before starting the build, returning `400 Bad Request` if not.

### Modified build function

`build_qol_tray_self_with_progress` gains an `Option<&Path>` parameter for the repo root:

```rust
pub fn build_qol_tray_self_with_progress<F>(
    repo_root: Option<&Path>,
    on_progress: F,
) -> BuildResult
```

When `None`, uses `PathBuf::from(env!("CARGO_MANIFEST_DIR"))` as before.

## Frontend state

New fields added to the dev view state in `use-controller.js`:

| Field              | Type              | Description                            |
|--------------------|-------------------|----------------------------------------|
| `worktrees`        | `array`           | fetched from `/api/dev/worktrees`      |
| `recompile`        | `object`          | `{ active, percent, phase, error }`    |
| `pickerOpen`       | `bool`            | chevron panel visibility               |

`defaultWorktree` is stored in `localStorage` under the key `dev.recompile.defaultWorktree`. The value is either `null` (main) or the absolute worktree path string. The UI derives the display branch name from the `worktrees` list.

SSE events handled in the dev view controller:

| Event                    | Action                              |
|--------------------------|-------------------------------------|
| `self_recompile_progress`| update `recompile.active/percent/phase` |
| `self_recompile_complete`| set `recompile.active=false, done=true`, auto-clear |
| `self_recompile_failed`  | set `recompile.error`               |

## Scope boundaries

- The sidebar footer recompile button is unchanged; it still fires against `CARGO_MANIFEST_DIR` (main).
- No new backend state — the default worktree is a frontend-only preference.
- The worktree list is fetched once on dev view mount; no polling.
