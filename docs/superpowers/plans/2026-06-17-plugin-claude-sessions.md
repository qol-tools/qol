# plugin-claude-sessions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a qol-tray plugin that shows an always-on-top panel of every live Claude Code session, colored by status, with Enter-to-jump via a pluggable terminal-host strategy (kitty first).

**Architecture:** One Rust crate, one binary that dispatches on argv: no-arg = a long-running daemon (GPUI panel + a hook-ingest stream socket + an action stream socket + a reconciler thread); `hook` = a fast shim Claude invokes per event that forwards one JSON line to the daemon; `open`/`cleanup` = actions dispatched to the running daemon. Status comes from Claude Code hooks the plugin self-installs (typed: `PermissionRequest`/`Notification.notification_type` -> red, `Stop`/idle -> yellow, prompt/tool -> green). The libproc session resolver and the kitty `@ ls` parser are lifted verbatim from git history (`86305fa5^`).

**Tech Stack:** Rust 2021, gpui + qol-gpui, qol-plugin-daemon, qol-runtime, qol-config, libproc (macOS), kitty remote control (`kitten @ ls` / `@ focus-window`), serde/serde_json.

**Design reference:** `docs/superpowers/specs/2026-06-17-claude-sessions-overview-design.md`.

**Spec delta to confirm during implementation:** the spec lists three statuses (green/yellow/red). Host `discover()` can surface a session before any hook fires, where the true status is unknown. This plan adds a fourth `Unknown` (grey) status for those cold rows rather than mislabel them green. If you prefer cold rows hidden until their first hook, drop `Unknown` and skip the cold-upsert in Task 11.

---

## File Structure

```
plugins/plugin-claude-sessions/
  Cargo.toml                 # package + workspace deps
  plugin.toml                # manifest: daemon, actions open/cleanup, one binary
  qol-config.toml            # settings UI schema
  qol-runtime.toml           # action descriptions
  src/
    main.rs                  # argv dispatch
    lib.rs                   # module tree + re-exports
    status.rs                # Status, HookEvent, map_event (pure)
    encoding.rs              # encode_cwd (lifted)
    registry.rs              # SessionState, Registry: upsert/prune/sorted (pure)
    hooks/
      mod.rs
      settings.rs           # managed-block build/merge/detect/remove + POSIX escape (pure)
      shim.rs               # `hook` subcommand: stdin -> event -> socket
      ingest.rs             # daemon-side hook-ingest stream socket
    host/
      mod.rs                # TerminalHost trait, Pane, join() (pure join logic)
      kitty/
        mod.rs              # kitty impl: discover()/focus() via `kitten`
        parse.rs            # lifted `parse_ls` + structs
    resolver/               # lifted libproc resolver (mod.rs + platform/*)
    pid.rs                  # parent_pid + walk_to_claude (libproc)
    git.rs                  # branch lookup (thin)
    daemon/
      mod.rs                # run(): GPUI bootstrap + spawn threads
      actions.rs            # action socket: Command, parse_command, start_listener
      reconcile.rs          # reconciler tick: prune + discover/join + branch + self-heal
    ui/
      mod.rs                # SessionsView struct + ctor
      render.rs             # Render impl (two-line tinted rows + keyboard)
      run.rs                # window open + command-loop wiring
  tests/
    status_mapping.rs
    registry.rs
    settings_manager.rs
    kitty_parse.rs
    manifest_structural.rs
```

Module boundaries: pure logic (`status`, `registry`, `hooks::settings`, `host::join`, `host::kitty::parse`, `encoding`) is I/O-free and unit-tested. I/O adapters (`resolver`, `pid`, `git`, `host::kitty`, sockets) are thin and verified by build + a smoke run. The daemon/UI compose them.

---

## Task 1: Crate scaffold and argv dispatch skeleton

**Files:**
- Create: `plugins/plugin-claude-sessions/Cargo.toml`
- Create: `plugins/plugin-claude-sessions/plugin.toml`
- Create: `plugins/plugin-claude-sessions/qol-config.toml`
- Create: `plugins/plugin-claude-sessions/qol-runtime.toml`
- Create: `plugins/plugin-claude-sessions/src/main.rs`
- Create: `plugins/plugin-claude-sessions/src/lib.rs`
- Modify: root `Cargo.toml` workspace `members` (add the crate if the workspace does not use a glob; check first)

- [ ] **Step 1: Create `Cargo.toml`**

```toml
[package]
name = "plugin-claude-sessions"
version = "0.1.0"
edition = "2021"
description = "Always-on-top overview of live Claude Code sessions for QoL Tray"
license = "PolyForm-Noncommercial-1.0.0"

[dependencies]
anyhow = "1"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1"
gpui.workspace = true
qol-gpui.workspace = true
qol-plugin-daemon.workspace = true
qol-runtime.workspace = true
qol-config.workspace = true

[target.'cfg(target_os = "macos")'.dependencies]
libc = "0.2"

[target.'cfg(target_os = "linux")'.dependencies]
libc = "0.2"

[dev-dependencies]
qol-plugin-api.workspace = true
serde_json = "1"
```

- [ ] **Step 2: Create `plugin.toml`**

```toml
[plugin]
id = "plugin-claude-sessions"
name = "Claude Sessions"
description = "Always-on-top overview of live Claude Code sessions"
version = "0.1.0"
author = ""
platforms = ["linux", "macos"]

[runtime]
command = "plugin-claude-sessions"
actions = { open = ["open"], cleanup = ["cleanup"] }

[daemon]
enabled = true
command = "plugin-claude-sessions"
socket = "/tmp/qol-claude-sessions.sock"

[capabilities]
gpui = true

[menu]
label = "Claude Sessions"
items = [
    { type = "action", id = "open", label = "Open Overview", action = "run" },
    { type = "action", id = "cleanup", label = "Remove Hooks", action = "run" },
]

[[dependencies.binaries]]
name = "plugin-claude-sessions"
repo = "qol-tools/plugin-claude-sessions"
pattern = "plugin-claude-sessions-{os}-{arch}"
```

- [ ] **Step 3: Create `qol-runtime.toml`**

```toml
schema_version = 1

[action.open]
description = "Show or focus the Claude sessions overview panel"

[action.cleanup]
description = "Remove the managed Claude Code hook block from ~/.claude/settings.json"
```

- [ ] **Step 4: Create `qol-config.toml`**

```toml
schema_version = 1
title = "Claude Sessions"
description = "Always-on-top overview of live Claude Code sessions."

[section.panel]
label = "Panel"
description = "Where the always-on-top panel sits and how it refreshes."

[field.corner]
type = "string"
section = "panel"
config_key = "corner"
label = "Screen corner"
description = "Which corner the panel parks in: top-left, top-right, bottom-left, bottom-right."
default = "top-right"

[field.poll_secs]
type = "number"
section = "panel"
config_key = "poll_secs"
label = "Refresh interval (seconds)"
description = "How often the reconciler prunes dead sessions and re-scans the terminal host."
default = 3

[field.host]
type = "string"
section = "panel"
config_key = "host"
label = "Terminal host"
description = "Which terminal the jump-to-session uses. Only 'kitty' is supported today."
default = "kitty"
```

- [ ] **Step 5: Create `src/lib.rs`**

```rust
pub mod encoding;
pub mod registry;
pub mod status;

pub mod host;
pub mod hooks;
```

(Other modules are added to `lib.rs` by their tasks as they are created.)

- [ ] **Step 6: Create `src/main.rs` with argv dispatch (kitty precedent)**

```rust
use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    match env::args().nth(1).as_deref() {
        None | Some("daemon") | Some("run") => ExitCode::SUCCESS,
        Some("hook") => ExitCode::SUCCESS,
        Some("open") => ExitCode::SUCCESS,
        Some("cleanup") => ExitCode::SUCCESS,
        Some(other) => {
            eprintln!("plugin-claude-sessions: unknown subcommand {other:?}");
            ExitCode::from(2)
        }
    }
}
```

- [ ] **Step 7: Register the crate in the workspace if needed**

Run: `grep -n "members" /Users/kaho/repos/private/qol-monorepo/Cargo.toml`
If `members` lists plugins explicitly (not a `plugins/*` glob), add `"plugins/plugin-claude-sessions"`. If it is a glob, no change.

- [ ] **Step 8: Build**

Run: `cargo build -p plugin-claude-sessions`
Expected: compiles clean (empty modules + dispatch skeleton).

- [ ] **Step 9: Commit**

```bash
git add plugins/plugin-claude-sessions Cargo.toml
git commit -m "feat(claude-sessions): scaffold plugin crate and argv dispatch"
```

---

## Task 2: Status model and hook-event mapping (pure)

**Files:**
- Create: `plugins/plugin-claude-sessions/src/status.rs`
- Test: `plugins/plugin-claude-sessions/tests/status_mapping.rs`

- [ ] **Step 1: Write the failing test**

```rust
use plugin_claude_sessions::status::{map_event, HookEvent, Mapped, Status};

fn ev(event: &str) -> HookEvent {
    HookEvent {
        session_id: "s1".into(),
        cwd: "/a/b/c".into(),
        transcript_path: None,
        event: event.into(),
        tool_name: None,
        notification_type: None,
        message: None,
    }
}

#[test]
fn maps_events_to_status_and_summary() {
    let cases = [
        ("UserPromptSubmit", Mapped::Set { status: Status::Working, summary: "working".into() }),
        ("SessionStart", Mapped::Set { status: Status::Working, summary: "started".into() }),
        ("PermissionRequest", Mapped::Set { status: Status::NeedsYou, summary: "permission".into() }),
        ("Stop", Mapped::Set { status: Status::YourTurn, summary: "your turn".into() }),
        ("SubagentStop", Mapped::Set { status: Status::YourTurn, summary: "your turn".into() }),
        ("SessionEnd", Mapped::Remove),
        ("PreCompact", Mapped::Ignore),
    ];
    for (event, expected) in cases {
        assert_eq!(map_event(&ev(event)), expected, "event: {event}");
    }
}

#[test]
fn pre_tool_use_uses_tool_name_as_summary() {
    let mut e = ev("PreToolUse");
    e.tool_name = Some("Bash".into());
    assert_eq!(
        map_event(&e),
        Mapped::Set { status: Status::Working, summary: "Bash".into() }
    );
}

#[test]
fn notification_is_typed() {
    let cases = [
        (Some("permission_prompt"), None, Mapped::Set { status: Status::NeedsYou, summary: "permission".into() }),
        (Some("idle_prompt"), None, Mapped::Set { status: Status::YourTurn, summary: "your turn".into() }),
        (Some("auth_success"), None, Mapped::Ignore),
        (None, Some("Claude needs your permission to run Bash"), Mapped::Set { status: Status::NeedsYou, summary: "permission".into() }),
        (None, Some("Waiting for your input"), Mapped::Set { status: Status::YourTurn, summary: "your turn".into() }),
    ];
    for (nt, msg, expected) in cases {
        let mut e = ev("Notification");
        e.notification_type = nt.map(str::to_string);
        e.message = msg.map(str::to_string);
        assert_eq!(map_event(&e), expected, "nt={nt:?} msg={msg:?}");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p plugin-claude-sessions --test status_mapping`
Expected: FAIL (module/types not defined).

- [ ] **Step 3: Implement `src/status.rs`**

```rust
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Working,
    YourTurn,
    NeedsYou,
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HookEvent {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(rename = "hook_event_name", default)]
    pub event: String,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub notification_type: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mapped {
    Set { status: Status, summary: String },
    Remove,
    Ignore,
}

fn set(status: Status, summary: &str) -> Mapped {
    Mapped::Set { status, summary: summary.to_string() }
}

pub fn map_event(ev: &HookEvent) -> Mapped {
    match ev.event.as_str() {
        "UserPromptSubmit" => set(Status::Working, "working"),
        "PreToolUse" => set(
            Status::Working,
            ev.tool_name.as_deref().unwrap_or("working"),
        ),
        "SessionStart" => set(Status::Working, "started"),
        "PermissionRequest" => set(Status::NeedsYou, "permission"),
        "Stop" | "SubagentStop" => set(Status::YourTurn, "your turn"),
        "Notification" => map_notification(ev),
        "SessionEnd" => Mapped::Remove,
        _ => Mapped::Ignore,
    }
}

fn map_notification(ev: &HookEvent) -> Mapped {
    if let Some(nt) = ev.notification_type.as_deref() {
        return match nt {
            "permission_prompt" => set(Status::NeedsYou, "permission"),
            "idle_prompt" => set(Status::YourTurn, "your turn"),
            _ => Mapped::Ignore,
        };
    }
    match ev.message.as_deref() {
        Some(m) if m.to_lowercase().contains("permission") => set(Status::NeedsYou, "permission"),
        Some(m) if m.to_lowercase().contains("waiting") => set(Status::YourTurn, "your turn"),
        _ => Mapped::Ignore,
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p plugin-claude-sessions --test status_mapping`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add plugins/plugin-claude-sessions/src/status.rs plugins/plugin-claude-sessions/tests/status_mapping.rs
git commit -m "feat(claude-sessions): typed hook-event to status mapping"
```

---

## Task 3: Lift `encode_cwd`

**Files:**
- Create: `plugins/plugin-claude-sessions/src/encoding.rs`

- [ ] **Step 1: Lift the module verbatim from history**

Run: `git show 86305fa5^:plugins/plugin-claude-sessions/src/encoding.rs > plugins/plugin-claude-sessions/src/encoding.rs`

This file is comment-light pure code: `pub fn encode_cwd(cwd: &Path) -> String` replacing `/` with `-`. The repo is comment-free; strip the doc comments after lifting.

- [ ] **Step 2: Strip doc comments to satisfy the no-comments rule**

Edit `src/encoding.rs` to:

```rust
use std::path::Path;

pub fn encode_cwd(cwd: &Path) -> String {
    cwd.to_string_lossy().replace('/', "-")
}
```

- [ ] **Step 3: Add a unit test at the bottom**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn encodes_absolute_path_to_leading_dash_form() {
        let cases = [
            ("/a/b/c", "-a-b-c"),
            ("/", "-"),
            ("/Users/x/repo", "-Users-x-repo"),
        ];
        for (input, expected) in cases {
            assert_eq!(encode_cwd(Path::new(input)), expected, "input: {input}");
        }
    }
}
```

- [ ] **Step 4: Run test**

Run: `cargo test -p plugin-claude-sessions encoding`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add plugins/plugin-claude-sessions/src/encoding.rs
git commit -m "feat(claude-sessions): lift encode_cwd helper"
```

---

## Task 4: Session registry (pure)

**Files:**
- Create: `plugins/plugin-claude-sessions/src/registry.rs`
- Test: `plugins/plugin-claude-sessions/tests/registry.rs`

- [ ] **Step 1: Write the failing test**

```rust
use plugin_claude_sessions::registry::{Registry, SessionState};
use plugin_claude_sessions::status::Status;

fn state(id: &str, status: Status, last: u64) -> SessionState {
    SessionState {
        session_id: id.into(),
        pid: 100,
        project: "proj".into(),
        cwd: "/a/b/proj".into(),
        branch: None,
        status,
        summary: "x".into(),
        last_activity: last,
    }
}

#[test]
fn upsert_is_last_writer_wins_by_session_id() {
    let mut r = Registry::default();
    r.upsert(state("s1", Status::Working, 1));
    r.upsert(state("s1", Status::NeedsYou, 2));
    let all = r.sorted();
    assert_eq!(all.len(), 1, "same session id merges");
    assert_eq!(all[0].status, Status::NeedsYou);
    assert_eq!(all[0].last_activity, 2);
}

#[test]
fn prune_removes_dead_pids() {
    let mut r = Registry::default();
    let mut alive = state("alive", Status::Working, 1);
    alive.pid = 1;
    let mut dead = state("dead", Status::Working, 1);
    dead.pid = 2;
    r.upsert(alive);
    r.upsert(dead);
    r.prune(|pid| pid == 1);
    let all = r.sorted();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].session_id, "alive");
}

#[test]
fn sorted_orders_red_yellow_green_then_recent() {
    let mut r = Registry::default();
    r.upsert(state("g_old", Status::Working, 1));
    r.upsert(state("g_new", Status::Working, 5));
    r.upsert(state("y", Status::YourTurn, 2));
    r.upsert(state("r", Status::NeedsYou, 1));
    let ids: Vec<_> = r.sorted().into_iter().map(|s| s.session_id).collect();
    assert_eq!(ids, vec!["r", "y", "g_new", "g_old"]);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p plugin-claude-sessions --test registry`
Expected: FAIL.

- [ ] **Step 3: Implement `src/registry.rs`**

```rust
use std::collections::HashMap;

use crate::status::Status;

#[derive(Debug, Clone)]
pub struct SessionState {
    pub session_id: String,
    pub pid: i32,
    pub project: String,
    pub cwd: String,
    pub branch: Option<String>,
    pub status: Status,
    pub summary: String,
    pub last_activity: u64,
}

#[derive(Default)]
pub struct Registry {
    sessions: HashMap<String, SessionState>,
}

impl Registry {
    pub fn upsert(&mut self, state: SessionState) {
        self.sessions.insert(state.session_id.clone(), state);
    }

    pub fn remove(&mut self, session_id: &str) {
        self.sessions.remove(session_id);
    }

    pub fn contains_pid(&self, pid: i32) -> bool {
        self.sessions.values().any(|s| s.pid == pid)
    }

    pub fn prune(&mut self, is_alive: impl Fn(i32) -> bool) {
        self.sessions.retain(|_, s| is_alive(s.pid));
    }

    pub fn sorted(&self) -> Vec<SessionState> {
        let mut out: Vec<SessionState> = self.sessions.values().cloned().collect();
        out.sort_by(|a, b| {
            rank(a.status)
                .cmp(&rank(b.status))
                .then(b.last_activity.cmp(&a.last_activity))
                .then(a.session_id.cmp(&b.session_id))
        });
        out
    }
}

fn rank(status: Status) -> u8 {
    match status {
        Status::NeedsYou => 0,
        Status::YourTurn => 1,
        Status::Working => 2,
        Status::Unknown => 3,
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p plugin-claude-sessions --test registry`
Expected: PASS.

- [ ] **Step 5: Add `pub mod registry;` to `lib.rs` if not already present, then commit**

```bash
git add plugins/plugin-claude-sessions/src/registry.rs plugins/plugin-claude-sessions/tests/registry.rs plugins/plugin-claude-sessions/src/lib.rs
git commit -m "feat(claude-sessions): session registry with prune and status sort"
```

---

## Task 5: settings.json manager (pure: escape + managed block)

**Files:**
- Create: `plugins/plugin-claude-sessions/src/hooks/mod.rs`
- Create: `plugins/plugin-claude-sessions/src/hooks/settings.rs`
- Test: `plugins/plugin-claude-sessions/tests/settings_manager.rs`

- [ ] **Step 1: Write the failing test**

```rust
use plugin_claude_sessions::hooks::settings::{
    install_block, managed_command, remove_block, shell_single_quote,
};
use serde_json::json;

const MARKER: &str = "qol-claude-sessions";

#[test]
fn single_quote_escapes_all_metacharacters() {
    let cases = [
        ("plain", "'plain'"),
        ("a b", "'a b'"),
        ("a'b", "'a'\\''b'"),
        ("$x`y;z\n", "'$x`y;z\n'"),
    ];
    for (input, expected) in cases {
        assert_eq!(shell_single_quote(input), expected, "input: {input:?}");
    }
}

#[test]
fn managed_command_is_existence_guarded_and_quoted() {
    let cmd = managed_command("/Apps/qol-tray/x sessions/bin", "/tmp/q.sock", MARKER);
    assert_eq!(
        cmd,
        "test -x '/Apps/qol-tray/x sessions/bin' && '/Apps/qol-tray/x sessions/bin' hook --marker qol-claude-sessions --socket '/tmp/q.sock' || true"
    );
}

#[test]
fn install_is_idempotent_and_preserves_foreign_hooks() {
    let mut settings = json!({
        "hooks": {
            "Stop": [ { "matcher": "*", "hooks": [ { "type": "command", "command": "user-own-hook" } ] } ]
        }
    });
    install_block(&mut settings, "/bin/pcs", "/tmp/q.sock", MARKER);
    let snapshot = settings.clone();
    install_block(&mut settings, "/bin/pcs", "/tmp/q.sock", MARKER);
    assert_eq!(settings, snapshot, "second install is a no-op");

    let stop = settings["hooks"]["Stop"].as_array().unwrap();
    assert!(
        stop.iter().any(|g| group_has_command(g, "user-own-hook")),
        "foreign Stop hook preserved"
    );
    assert!(
        stop.iter().any(|g| group_has_marker(g, MARKER)),
        "managed Stop hook present"
    );
}

#[test]
fn remove_strips_only_managed_entries() {
    let mut settings = json!({
        "hooks": {
            "Stop": [ { "matcher": "*", "hooks": [ { "type": "command", "command": "user-own-hook" } ] } ]
        }
    });
    install_block(&mut settings, "/bin/pcs", "/tmp/q.sock", MARKER);
    remove_block(&mut settings, MARKER);
    let stop = settings["hooks"]["Stop"].as_array().unwrap();
    assert!(stop.iter().any(|g| group_has_command(g, "user-own-hook")));
    assert!(!stop.iter().any(|g| group_has_marker(g, MARKER)));
}

fn group_has_command(group: &serde_json::Value, needle: &str) -> bool {
    group["hooks"].as_array().map_or(false, |hs| {
        hs.iter().any(|h| h["command"].as_str() == Some(needle))
    })
}

fn group_has_marker(group: &serde_json::Value, marker: &str) -> bool {
    group["hooks"].as_array().map_or(false, |hs| {
        hs.iter().any(|h| {
            h["command"].as_str().map_or(false, |c| c.contains(marker))
        })
    })
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p plugin-claude-sessions --test settings_manager`
Expected: FAIL.

- [ ] **Step 3: Implement `src/hooks/mod.rs`**

```rust
pub mod settings;
```

- [ ] **Step 4: Implement `src/hooks/settings.rs`**

```rust
use serde_json::{json, Map, Value};

pub const EVENTS: &[&str] = &[
    "UserPromptSubmit",
    "PreToolUse",
    "SessionStart",
    "SessionEnd",
    "Stop",
    "SubagentStop",
    "Notification",
    "PermissionRequest",
];

pub fn shell_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

pub fn managed_command(bin: &str, socket: &str, marker: &str) -> String {
    let b = shell_single_quote(bin);
    let s = shell_single_quote(socket);
    format!("test -x {b} && {b} hook --marker {marker} --socket {s} || true")
}

fn managed_group(bin: &str, socket: &str, marker: &str) -> Value {
    json!({
        "matcher": "*",
        "hooks": [ { "type": "command", "command": managed_command(bin, socket, marker) } ]
    })
}

fn group_is_managed(group: &Value, marker: &str) -> bool {
    group["hooks"].as_array().map_or(false, |hooks| {
        hooks.iter().any(|h| {
            h["command"]
                .as_str()
                .map_or(false, |c| c.contains(&format!("--marker {marker}")))
        })
    })
}

pub fn install_block(settings: &mut Value, bin: &str, socket: &str, marker: &str) {
    let root = ensure_object(settings);
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()));
    let hooks = match hooks.as_object_mut() {
        Some(h) => h,
        None => {
            *hooks = Value::Object(Map::new());
            hooks.as_object_mut().expect("just set to object")
        }
    };
    for event in EVENTS {
        let arr = hooks
            .entry((*event).to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        let arr = match arr.as_array_mut() {
            Some(a) => a,
            None => continue,
        };
        arr.retain(|g| !group_is_managed(g, marker));
        arr.push(managed_group(bin, socket, marker));
    }
}

pub fn remove_block(settings: &mut Value, marker: &str) {
    let Some(hooks) = settings
        .get_mut("hooks")
        .and_then(|h| h.as_object_mut())
    else {
        return;
    };
    for (_event, arr) in hooks.iter_mut() {
        if let Some(arr) = arr.as_array_mut() {
            arr.retain(|g| !group_is_managed(g, marker));
        }
    }
}

fn ensure_object(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value.as_object_mut().expect("just ensured object")
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p plugin-claude-sessions --test settings_manager`
Expected: PASS. (Note: the idempotency test relies on `retain` then `push`, which replaces the managed group in place, so a second install yields an identical document.)

- [ ] **Step 6: Add `pub mod hooks;` to `lib.rs` (Task 1 already added it) and commit**

```bash
git add plugins/plugin-claude-sessions/src/hooks plugins/plugin-claude-sessions/tests/settings_manager.rs
git commit -m "feat(claude-sessions): managed settings.json hook block with POSIX-safe command"
```

---

## Task 6: Lift the kitty `@ ls` parser

**Files:**
- Create: `plugins/plugin-claude-sessions/src/host/mod.rs`
- Create: `plugins/plugin-claude-sessions/src/host/kitty/mod.rs`
- Create: `plugins/plugin-claude-sessions/src/host/kitty/parse.rs`
- Test: `plugins/plugin-claude-sessions/tests/kitty_parse.rs`

- [ ] **Step 1: Lift the parser**

Copy the `parse_ls` parser and its `KittyLs`/`OsWindow`/`Tab`/`KittyWindow`/`ForegroundProcess` structs from `86305fa5^:plugins/plugin-kitty/src/kitty.rs` into `src/host/kitty/parse.rs`. Keep only the `@ ls` parser half (the `LaunchType`/`build_launch_argv`/`BinaryVerifier` halves are not needed). Strip doc comments. The retained surface is:

```rust
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct KittyLs(pub Vec<OsWindow>);

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OsWindow {
    pub id: u64,
    pub tabs: Vec<Tab>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Tab {
    pub id: u64,
    #[serde(default)]
    pub layout: String,
    pub windows: Vec<KittyWindow>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct KittyWindow {
    pub id: u64,
    pub title: String,
    pub cwd: PathBuf,
    #[serde(default)]
    pub foreground_processes: Vec<ForegroundProcess>,
}

impl KittyWindow {
    pub fn foreground_cmdline(&self) -> Option<&[String]> {
        self.foreground_processes.last().map(|p| p.cmdline.as_slice())
    }

    pub fn foreground_pid(&self) -> Option<u32> {
        self.foreground_processes.last().map(|p| p.pid)
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ForegroundProcess {
    pub pid: u32,
    pub cmdline: Vec<String>,
}

impl KittyLs {
    pub fn windows(&self) -> Vec<&KittyWindow> {
        self.0
            .iter()
            .flat_map(|os| os.tabs.iter().flat_map(|t| t.windows.iter()))
            .collect()
    }
}

#[derive(Debug)]
pub enum LsParseError {
    Json(serde_json::Error),
}

impl std::fmt::Display for LsParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LsParseError::Json(e) => write!(f, "kitty @ ls JSON parse error: {e}"),
        }
    }
}

impl std::error::Error for LsParseError {}

pub fn parse_ls(body: &str) -> Result<KittyLs, LsParseError> {
    serde_json::from_str(body).map_err(LsParseError::Json)
}
```

- [ ] **Step 2: Create `src/host/kitty/mod.rs` (parser exposed for now)**

```rust
pub mod parse;
```

- [ ] **Step 3: Create `src/host/mod.rs` (placeholder; trait added in Task 7)**

```rust
pub mod kitty;
```

- [ ] **Step 4: Write the parse test**

```rust
use plugin_claude_sessions::host::kitty::parse::parse_ls;

const SAMPLE: &str = r#"
[ { "id": 1, "tabs": [ { "id": 1, "layout": "tall", "windows": [
  { "id": 10, "title": "claude - proj", "cwd": "/a/b/proj",
    "foreground_processes": [ { "pid": 4242, "cmdline": ["node", "claude"] } ] },
  { "id": 11, "title": "shell", "cwd": "/a/b",
    "foreground_processes": [ { "pid": 9, "cmdline": ["-zsh"] } ] }
] } ] } ]
"#;

#[test]
fn flattens_windows_and_reads_foreground() {
    let ls = parse_ls(SAMPLE).expect("parse");
    let windows = ls.windows();
    assert_eq!(windows.len(), 2);
    let claude = windows[0];
    assert_eq!(claude.foreground_pid(), Some(4242));
    assert_eq!(claude.cwd.to_str(), Some("/a/b/proj"));
    let last = claude.foreground_cmdline().unwrap().last().unwrap();
    assert_eq!(last, "claude");
}
```

- [ ] **Step 5: Run test**

Run: `cargo test -p plugin-claude-sessions --test kitty_parse`
Expected: PASS.

- [ ] **Step 6: Add `pub mod host;` to `lib.rs` (Task 1 added it) and commit**

```bash
git add plugins/plugin-claude-sessions/src/host plugins/plugin-claude-sessions/tests/kitty_parse.rs
git commit -m "feat(claude-sessions): lift kitty @ ls parser"
```

---

## Task 7: Host trait, Pane, and cold-session join (pure)

**Files:**
- Modify: `plugins/plugin-claude-sessions/src/host/mod.rs`
- Test: add `tests/host_join.rs`

- [ ] **Step 1: Write the failing test**

```rust
use plugin_claude_sessions::host::{join_cold, Pane};
use plugin_claude_sessions::registry::Registry;
use plugin_claude_sessions::status::Status;

#[test]
fn join_adds_unknown_rows_only_for_unseen_claude_pids() {
    let mut r = Registry::default();
    // a pane whose pid is already tracked must not be duplicated
    let panes = vec![
        Pane { pid: 100, cwd: "/a/known".into(), title: "claude".into() },
        Pane { pid: 200, cwd: "/a/cold".into(), title: "claude - cold".into() },
    ];
    // pretend 100 already exists via a prior hook
    r.upsert(plugin_claude_sessions::registry::SessionState {
        session_id: "known".into(), pid: 100, project: "known".into(),
        cwd: "/a/known".into(), branch: None, status: Status::Working,
        summary: "working".into(), last_activity: 9,
    });
    join_cold(&mut r, &panes, |pid, cwd| Some(format!("sess-{pid}-{}", cwd.len())), 5);
    let sorted = r.sorted();
    assert_eq!(sorted.len(), 2, "cold pane added, known pane not duplicated");
    let cold = sorted.iter().find(|s| s.pid == 200).unwrap();
    assert_eq!(cold.status, Status::Unknown);
    assert_eq!(cold.project, "cold");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p plugin-claude-sessions --test host_join`
Expected: FAIL.

- [ ] **Step 3: Implement the trait + join in `src/host/mod.rs`**

```rust
pub mod kitty;

use crate::registry::{Registry, SessionState};
use crate::status::Status;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pane {
    pub pid: i32,
    pub cwd: String,
    pub title: String,
}

pub trait TerminalHost {
    fn discover(&self) -> Vec<Pane>;
    fn focus(&self, pid: i32) -> anyhow::Result<()>;
}

fn project_of(cwd: &str) -> String {
    cwd.rsplit('/').find(|s| !s.is_empty()).unwrap_or(cwd).to_string()
}

pub fn join_cold(
    registry: &mut Registry,
    panes: &[Pane],
    resolve_session: impl Fn(i32, &str) -> Option<String>,
    now: u64,
) {
    for pane in panes {
        if registry.contains_pid(pane.pid) {
            continue;
        }
        let Some(session_id) = resolve_session(pane.pid, &pane.cwd) else {
            continue;
        };
        registry.upsert(SessionState {
            session_id,
            pid: pane.pid,
            project: project_of(&pane.cwd),
            cwd: pane.cwd.clone(),
            branch: None,
            status: Status::Unknown,
            summary: "(discovered)".into(),
            last_activity: now,
        });
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p plugin-claude-sessions --test host_join`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add plugins/plugin-claude-sessions/src/host/mod.rs plugins/plugin-claude-sessions/tests/host_join.rs
git commit -m "feat(claude-sessions): host trait and cold-session join"
```

---

## Task 8: Lift the libproc session resolver

**Files:**
- Create: `plugins/plugin-claude-sessions/src/resolver/mod.rs`
- Create: `plugins/plugin-claude-sessions/src/resolver/platform/{mod,macos,linux,windows}.rs`

- [ ] **Step 1: Lift the resolver tree verbatim**

```bash
mkdir -p plugins/plugin-claude-sessions/src/resolver/platform
git show 86305fa5^:plugins/plugin-claude-sessions/src/resolver/mod.rs > plugins/plugin-claude-sessions/src/resolver/mod.rs
git show 86305fa5^:plugins/plugin-claude-sessions/src/resolver/platform/mod.rs > plugins/plugin-claude-sessions/src/resolver/platform/mod.rs
git show 86305fa5^:plugins/plugin-claude-sessions/src/resolver/platform/macos.rs > plugins/plugin-claude-sessions/src/resolver/platform/macos.rs
git show 86305fa5^:plugins/plugin-claude-sessions/src/resolver/platform/linux.rs > plugins/plugin-claude-sessions/src/resolver/platform/linux.rs
git show 86305fa5^:plugins/plugin-claude-sessions/src/resolver/platform/windows.rs > plugins/plugin-claude-sessions/src/resolver/platform/windows.rs
```

- [ ] **Step 2: Strip doc comments** to satisfy the no-comments rule (logic unchanged). The public entry stays `resolver::resolve_session_jsonl(pid: u32, exe: &str) -> Result<PathBuf, ResolveError>`.

- [ ] **Step 3: Add a `session_id_from_pid` convenience that returns the jsonl file stem**

Append to `src/resolver/mod.rs`:

```rust
pub fn session_id_from_pid(pid: u32) -> Option<String> {
    let path = resolve_session_jsonl(pid, "claude").ok()?;
    path.file_stem()?.to_str().map(|s| s.to_string())
}
```

- [ ] **Step 4: Add `pub mod resolver;` to `lib.rs`. Lift the archived structural tests**

```bash
git show 86305fa5^:plugins/plugin-claude-sessions/tests/resolver_structural.rs > plugins/plugin-claude-sessions/tests/resolver_structural.rs
git show 86305fa5^:plugins/plugin-claude-sessions/tests/encoded_cwd_structural.rs > plugins/plugin-claude-sessions/tests/encoded_cwd_structural.rs
```
Adjust the crate name in `use` lines to `plugin_claude_sessions`.

- [ ] **Step 5: Build for all targets and run tests**

Run: `cargo build -p plugin-claude-sessions && cargo test -p plugin-claude-sessions resolver`
Expected: builds and tests pass on the host OS.

- [ ] **Step 6: Cross-compile check (warnings are errors on every backend)**

Run: `RUSTFLAGS="-D warnings" cargo clippy -p plugin-claude-sessions --all-targets`
Expected: clean. If Linux/Windows arms have unused symbols, gate them with `#[cfg]`.

- [ ] **Step 7: Commit**

```bash
git add plugins/plugin-claude-sessions/src/resolver plugins/plugin-claude-sessions/tests/resolver_structural.rs plugins/plugin-claude-sessions/tests/encoded_cwd_structural.rs plugins/plugin-claude-sessions/src/lib.rs
git commit -m "feat(claude-sessions): lift libproc session resolver"
```

---

## Task 9: parent-PID walk to the claude ancestor

**Files:**
- Create: `plugins/plugin-claude-sessions/src/pid.rs`

- [ ] **Step 1: Implement `parent_pid` + `walk_to_claude`**

Reuse the existing `parent_pid` pattern (already in `plugins/plugin-alt-tab/src/discovery/macos/process.rs` and `libs/qol-app-icon/src/macos.rs`). Lift the `proc_pidinfo` + `ProcBsdInfo` block and expose:

```rust
#[cfg(target_os = "macos")]
mod imp {
    // proc_pidinfo + ProcBsdInfo lifted from libs/qol-app-icon/src/macos.rs
    // exposes: pub fn parent_pid(pid: i32) -> Option<i32>
    //          pub fn exe_basename(pid: i32) -> Option<String>
}

#[cfg(target_os = "linux")]
mod imp {
    use std::fs;
    pub fn parent_pid(pid: i32) -> Option<i32> {
        let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        status
            .lines()
            .find_map(|l| l.strip_prefix("PPid:"))
            .and_then(|v| v.trim().parse().ok())
    }
    pub fn exe_basename(pid: i32) -> Option<String> {
        let link = fs::read_link(format!("/proc/{pid}/exe")).ok()?;
        link.file_name()?.to_str().map(|s| s.to_string())
    }
}

pub use imp::{exe_basename, parent_pid};

pub fn walk_to_claude(start: i32) -> Option<i32> {
    let mut pid = start;
    for _ in 0..8 {
        if pid <= 1 {
            return None;
        }
        if exe_basename(pid).as_deref() == Some("claude") {
            return Some(pid);
        }
        pid = parent_pid(pid)?;
    }
    None
}
```

- [ ] **Step 2: Add `pub mod pid;` to `lib.rs`. Build + clippy on host**

Run: `RUSTFLAGS="-D warnings" cargo clippy -p plugin-claude-sessions --all-targets`
Expected: clean. (No unit test: thin OS wrappers; covered by the smoke run in Task 17.)

- [ ] **Step 3: Commit**

```bash
git add plugins/plugin-claude-sessions/src/pid.rs plugins/plugin-claude-sessions/src/lib.rs
git commit -m "feat(claude-sessions): parent-pid walk to claude ancestor"
```

---

## Task 10: kitty host implementation (discover/focus)

**Files:**
- Modify: `plugins/plugin-claude-sessions/src/host/kitty/mod.rs`

- [ ] **Step 1: Implement the kitty host**

```rust
pub mod parse;

use std::process::Command;

use crate::host::{Pane, TerminalHost};
use parse::parse_ls;

pub struct Kitty;

fn is_claude(window: &parse::KittyWindow) -> bool {
    window
        .foreground_cmdline()
        .and_then(|c| c.last())
        .map(|last| {
            std::path::Path::new(last)
                .file_name()
                .and_then(|n| n.to_str())
                == Some("claude")
        })
        .unwrap_or(false)
}

impl TerminalHost for Kitty {
    fn discover(&self) -> Vec<Pane> {
        let Ok(out) = Command::new("kitten")
            .args(["@", "ls", "--format=json"])
            .output()
        else {
            return Vec::new();
        };
        if !out.status.success() {
            return Vec::new();
        }
        let Ok(body) = String::from_utf8(out.stdout) else {
            return Vec::new();
        };
        let Ok(ls) = parse_ls(&body) else {
            return Vec::new();
        };
        ls.windows()
            .into_iter()
            .filter(|w| is_claude(w))
            .filter_map(|w| {
                Some(Pane {
                    pid: w.foreground_pid()? as i32,
                    cwd: w.cwd.to_string_lossy().into_owned(),
                    title: w.title.clone(),
                })
            })
            .collect()
    }

    fn focus(&self, pid: i32) -> anyhow::Result<()> {
        let status = Command::new("kitten")
            .args(["@", "focus-window", "--match", &format!("pid:{pid}")])
            .status()?;
        anyhow::ensure!(status.success(), "kitten @ focus-window failed");
        Ok(())
    }
}
```

- [ ] **Step 2: Build + clippy**

Run: `RUSTFLAGS="-D warnings" cargo clippy -p plugin-claude-sessions --all-targets`
Expected: clean. (No unit test: thin process wrapper. `is_claude` is covered indirectly by the parse test data; if desired, add a 3-row table test for `is_claude`.)

- [ ] **Step 3: Commit**

```bash
git add plugins/plugin-claude-sessions/src/host/kitty/mod.rs
git commit -m "feat(claude-sessions): kitty discover and focus over remote control"
```

---

## Task 11: git branch helper

**Files:**
- Create: `plugins/plugin-claude-sessions/src/git.rs`

- [ ] **Step 1: Implement**

```rust
use std::process::Command;

pub fn branch(cwd: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(cwd)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!s.is_empty() && s != "HEAD").then_some(s)
}
```

- [ ] **Step 2: Add `pub mod git;` to `lib.rs`. Build + clippy**

Run: `RUSTFLAGS="-D warnings" cargo clippy -p plugin-claude-sessions --all-targets`
Expected: clean. (Thin wrapper; no unit test.)

- [ ] **Step 3: Commit**

```bash
git add plugins/plugin-claude-sessions/src/git.rs plugins/plugin-claude-sessions/src/lib.rs
git commit -m "feat(claude-sessions): git branch lookup"
```

---

## Task 12: hook ingest socket protocol (shared types + daemon listener)

**Files:**
- Create: `plugins/plugin-claude-sessions/src/hooks/ingest.rs`
- Modify: `plugins/plugin-claude-sessions/src/hooks/mod.rs`
- Test: `tests/ingest_protocol.rs`

- [ ] **Step 1: Write the failing test (line protocol round-trip)**

```rust
use plugin_claude_sessions::hooks::ingest::{decode_line, encode_line, IngestMsg};
use plugin_claude_sessions::status::Status;

#[test]
fn ingest_line_round_trips() {
    let msg = IngestMsg {
        session_id: "s1".into(),
        pid: 4242,
        cwd: "/a/b/proj".into(),
        status: Status::NeedsYou,
        summary: "permission".into(),
        remove: false,
    };
    let line = encode_line(&msg);
    assert!(line.ends_with('\n'));
    let back = decode_line(line.trim()).expect("decode");
    assert_eq!(back.session_id, "s1");
    assert_eq!(back.pid, 4242);
    assert_eq!(back.status, Status::NeedsYou);
    assert!(!back.remove);
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p plugin-claude-sessions --test ingest_protocol`
Expected: FAIL.

- [ ] **Step 3: Implement `src/hooks/ingest.rs`**

```rust
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixListener;
use std::sync::mpsc::Sender;

use serde::{Deserialize, Serialize};

use crate::status::Status;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestMsg {
    pub session_id: String,
    pub pid: i32,
    pub cwd: String,
    pub status: StatusWire,
    pub summary: String,
    #[serde(default)]
    pub remove: bool,
}

pub type StatusWire = Status;

pub fn encode_line(msg: &IngestMsg) -> String {
    let mut s = serde_json::to_string(msg).unwrap_or_default();
    s.push('\n');
    s
}

pub fn decode_line(line: &str) -> Option<IngestMsg> {
    serde_json::from_str(line).ok()
}

pub fn start_ingest(socket_path: &str, tx: Sender<IngestMsg>) -> std::io::Result<()> {
    let _ = std::fs::remove_file(socket_path);
    let listener = UnixListener::bind(socket_path)?;
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let reader = BufReader::new(stream);
            for line in reader.lines().map_while(Result::ok) {
                if let Some(msg) = decode_line(line.trim()) {
                    let _ = tx.send(msg);
                }
            }
        }
    });
    Ok(())
}
```

Add `#[derive(Serialize, Deserialize)]` to `Status` in `src/status.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Status { Working, YourTurn, NeedsYou, Unknown }
```

Add to `src/hooks/mod.rs`:

```rust
pub mod ingest;
pub mod settings;
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p plugin-claude-sessions --test ingest_protocol`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add plugins/plugin-claude-sessions/src/hooks plugins/plugin-claude-sessions/src/status.rs plugins/plugin-claude-sessions/tests/ingest_protocol.rs
git commit -m "feat(claude-sessions): hook-ingest socket protocol and listener"
```

---

## Task 13: `hook` subcommand (the shim)

**Files:**
- Create: `plugins/plugin-claude-sessions/src/hooks/shim.rs`
- Modify: `plugins/plugin-claude-sessions/src/hooks/mod.rs`, `src/main.rs`

- [ ] **Step 1: Implement `src/hooks/shim.rs`**

```rust
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

use crate::hooks::ingest::{encode_line, IngestMsg};
use crate::pid::walk_to_claude;
use crate::status::{map_event, HookEvent, Mapped};

pub fn run(socket_path: &str) {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return;
    }
    let Ok(event) = serde_json::from_str::<HookEvent>(&input) else {
        return;
    };
    let (status, summary, remove) = match map_event(&event) {
        Mapped::Set { status, summary } => (status, summary, false),
        Mapped::Remove => (crate::status::Status::Unknown, String::new(), true),
        Mapped::Ignore => return,
    };
    let pid = walk_to_claude(unsafe { libc::getppid() }).unwrap_or(0);
    let msg = IngestMsg {
        session_id: event.session_id,
        pid,
        cwd: event.cwd,
        status,
        summary,
        remove,
    };
    if let Ok(mut stream) = UnixStream::connect(socket_path) {
        let _ = stream.write_all(encode_line(&msg).as_bytes());
    }
}
```

Add `pub mod shim;` to `src/hooks/mod.rs`.

- [ ] **Step 2: Parse `--socket` in `main.rs` and dispatch `hook`**

```rust
use std::env;
use std::process::ExitCode;

use plugin_claude_sessions::hooks::shim;

fn flag_value(name: &str) -> Option<String> {
    let mut args = env::args();
    while let Some(a) = args.next() {
        if a == name {
            return args.next();
        }
    }
    None
}

fn main() -> ExitCode {
    match env::args().nth(1).as_deref() {
        Some("hook") => {
            let socket = flag_value("--socket").unwrap_or_default();
            shim::run(&socket);
            ExitCode::SUCCESS
        }
        None | Some("daemon") | Some("run") => ExitCode::SUCCESS,
        Some("open") => ExitCode::SUCCESS,
        Some("cleanup") => ExitCode::SUCCESS,
        Some(other) => {
            eprintln!("plugin-claude-sessions: unknown subcommand {other:?}");
            ExitCode::from(2)
        }
    }
}
```

Add `libc` to non-test deps (already added for both targets in Task 1).

- [ ] **Step 3: Build + clippy**

Run: `RUSTFLAGS="-D warnings" cargo clippy -p plugin-claude-sessions --all-targets`
Expected: clean.

- [ ] **Step 4: Manual smoke (optional)**

Run: `echo '{"hook_event_name":"Stop","session_id":"x","cwd":"/tmp"}' | cargo run -q -p plugin-claude-sessions -- hook --socket /tmp/does-not-exist.sock`
Expected: exits 0, no panic (socket absent path).

- [ ] **Step 5: Commit**

```bash
git add plugins/plugin-claude-sessions/src/hooks/shim.rs plugins/plugin-claude-sessions/src/hooks/mod.rs plugins/plugin-claude-sessions/src/main.rs
git commit -m "feat(claude-sessions): hook shim forwards events to the daemon"
```

---

## Task 14: action socket (Command, parser, listener)

**Files:**
- Create: `plugins/plugin-claude-sessions/src/daemon/mod.rs`
- Create: `plugins/plugin-claude-sessions/src/daemon/actions.rs`

- [ ] **Step 1: Implement `src/daemon/actions.rs`**

```rust
use std::sync::mpsc::Sender;

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult};

pub const CONFIG: DaemonConfig = DaemonConfig {
    default_socket_name: "qol-claude-sessions.sock",
    use_tmpdir_env: false,
    support_replace_existing: true,
};

#[derive(Debug)]
pub enum Command {
    Open,
    Cleanup,
    Kill,
}

fn parse_command(cmd: &str) -> ReadResult<Command> {
    match cmd {
        "ping" => ReadResult::Handled,
        "open" | "show" => ReadResult::Command(Command::Open),
        "cleanup" => ReadResult::Command(Command::Cleanup),
        "kill" => ReadResult::Command(Command::Kill),
        _ => ReadResult::Fallback,
    }
}

pub fn start_listener(tx: Sender<Command>) -> bool {
    core_daemon::start_listener(&CONFIG, tx, parse_command)
}
```

(Confirm the exact `DaemonConfig` field set against `libs/qol-plugin-daemon/src/daemon.rs`; the launcher uses `default_socket_name`, `use_tmpdir_env`, `support_replace_existing`. The manifest `socket = "/tmp/qol-claude-sessions.sock"` must resolve to the same path `default_socket_name` produces with `use_tmpdir_env=false`.)

- [ ] **Step 2: Implement `src/daemon/mod.rs` stub**

```rust
pub mod actions;
pub mod reconcile;

pub fn run() -> anyhow::Result<()> {
    Ok(())
}
```

- [ ] **Step 3: Create `src/daemon/reconcile.rs` stub (filled in Task 15)**

```rust
```

- [ ] **Step 4: Add `pub mod daemon;` to `lib.rs`. Build + clippy**

Run: `RUSTFLAGS="-D warnings" cargo clippy -p plugin-claude-sessions --all-targets`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add plugins/plugin-claude-sessions/src/daemon plugins/plugin-claude-sessions/src/lib.rs
git commit -m "feat(claude-sessions): action socket command parser"
```

---

## Task 15: reconciler + settings paths + cleanup wiring

**Files:**
- Modify: `plugins/plugin-claude-sessions/src/daemon/reconcile.rs`
- Create: `plugins/plugin-claude-sessions/src/paths.rs`
- Test: `tests/paths.rs`

- [ ] **Step 1: Write the failing test for the path/marker helpers**

```rust
use plugin_claude_sessions::paths::{hook_socket_path, MARKER};

#[test]
fn hook_socket_is_short_and_distinct_from_action_socket() {
    let p = hook_socket_path();
    assert!(p.starts_with("/tmp/"), "short path for AF_UNIX sun_path: {p}");
    assert!(p.ends_with("-hook.sock"));
    assert_ne!(p, "/tmp/qol-claude-sessions.sock");
    assert_eq!(MARKER, "qol-claude-sessions");
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p plugin-claude-sessions --test paths`
Expected: FAIL.

- [ ] **Step 3: Implement `src/paths.rs`**

```rust
pub const MARKER: &str = "qol-claude-sessions";

pub fn action_socket_path() -> String {
    "/tmp/qol-claude-sessions.sock".to_string()
}

pub fn hook_socket_path() -> String {
    "/tmp/qol-claude-sessions-hook.sock".to_string()
}

pub fn settings_json_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(std::path::PathBuf::from(home).join(".claude").join("settings.json"))
}

pub fn self_bin_path() -> Option<String> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
}
```

- [ ] **Step 4: Implement `src/daemon/reconcile.rs`**

```rust
use std::sync::{Arc, Mutex};

use crate::git;
use crate::host::{join_cold, TerminalHost};
use crate::hooks::settings;
use crate::paths;
use crate::registry::Registry;
use crate::resolver::session_id_from_pid;

pub fn ensure_hooks_installed() {
    let (Some(path), Some(bin)) = (paths::settings_json_path(), paths::self_bin_path()) else {
        return;
    };
    let mut value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let before = value.clone();
    settings::install_block(&mut value, &bin, &paths::hook_socket_path(), paths::MARKER);
    if value != before {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(&value) {
            let _ = std::fs::write(&path, text);
        }
    }
}

pub fn remove_hooks() {
    let Some(path) = paths::settings_json_path() else { return };
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    let Ok(mut value) = serde_json::from_str(&text) else { return };
    settings::remove_block(&mut value, paths::MARKER);
    if let Ok(out) = serde_json::to_string_pretty(&value) {
        let _ = std::fs::write(&path, out);
    }
}

fn pid_alive(pid: i32) -> bool {
    pid > 0 && unsafe { libc::kill(pid, 0) } == 0
}

pub fn tick(registry: &Arc<Mutex<Registry>>, host: &dyn TerminalHost, now: u64) {
    let panes = host.discover();
    let Ok(mut reg) = registry.lock() else { return };
    reg.prune(pid_alive);
    join_cold(&mut reg, &panes, |pid, _cwd| session_id_from_pid(pid as u32), now);
    let sessions = reg.sorted();
    drop(reg);
    let branches: Vec<(String, Option<String>)> = sessions
        .iter()
        .map(|s| (s.session_id.clone(), git::branch(&s.cwd)))
        .collect();
    if let Ok(mut reg) = registry.lock() {
        for (id, branch) in branches {
            if let Some(s) = reg.get_mut(&id) {
                s.branch = branch;
            }
        }
    }
}
```

Add `pub fn get_mut(&mut self, id: &str) -> Option<&mut SessionState>` to `Registry` (returns `self.sessions.get_mut(id)`).

- [ ] **Step 5: Run path test + build + clippy**

Run: `cargo test -p plugin-claude-sessions --test paths && RUSTFLAGS="-D warnings" cargo clippy -p plugin-claude-sessions --all-targets`
Expected: PASS + clean.

- [ ] **Step 6: Commit**

```bash
git add plugins/plugin-claude-sessions/src/paths.rs plugins/plugin-claude-sessions/src/daemon/reconcile.rs plugins/plugin-claude-sessions/src/registry.rs plugins/plugin-claude-sessions/src/lib.rs plugins/plugin-claude-sessions/tests/paths.rs
git commit -m "feat(claude-sessions): reconciler, settings paths, and hook install/remove"
```

---

## Task 16: UI view and two-line tinted rows

**Files:**
- Create: `plugins/plugin-claude-sessions/src/ui/mod.rs`
- Create: `plugins/plugin-claude-sessions/src/ui/render.rs`

Model the view on `plugins/plugin-launcher/src/ui/mod.rs` (struct + ctor) and `render.rs` (Render impl, `div()` element tree). Concrete deltas below.

- [ ] **Step 1: Implement `src/ui/mod.rs`**

```rust
use std::sync::{Arc, Mutex};

use gpui::{Context, FocusHandle, Focusable};

use crate::host::TerminalHost;
use crate::registry::Registry;

pub struct SessionsView {
    pub(crate) registry: Arc<Mutex<Registry>>,
    pub(crate) host: Arc<dyn TerminalHost + Send + Sync>,
    pub(crate) selected: usize,
    pub(crate) focus_handle: FocusHandle,
}

impl SessionsView {
    pub fn new(
        registry: Arc<Mutex<Registry>>,
        host: Arc<dyn TerminalHost + Send + Sync>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            registry,
            host,
            selected: 0,
            focus_handle: cx.focus_handle(),
        }
    }

    pub(crate) fn rows(&self) -> Vec<crate::registry::SessionState> {
        self.registry.lock().map(|r| r.sorted()).unwrap_or_default()
    }

    pub(crate) fn focus_selected(&self) {
        let rows = self.rows();
        if let Some(row) = rows.get(self.selected) {
            let _ = self.host.focus(row.pid);
        }
    }
}

impl Focusable for SessionsView {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
```

(Mark `Kitty` as `Send + Sync` - it is a unit struct, so add nothing; the `Arc<dyn TerminalHost + Send + Sync>` requires the trait object bound. Update the trait usage accordingly.)

- [ ] **Step 2: Implement `src/ui/render.rs`**

Render a fixed-width column of two-line rows. Status color maps: NeedsYou `#f85149`, YourTurn `#d29922`, Working `#3fb950`, Unknown `#6e7681`. Row tint is the status color at low alpha. Use the gpui builder pattern from `plugin-launcher/src/ui/render.rs`.

```rust
use gpui::{
    div, prelude::*, px, rgb, rgba, Context, Window,
};

use crate::registry::SessionState;
use crate::status::Status;
use crate::ui::SessionsView;

fn dot(status: Status) -> u32 {
    match status {
        Status::NeedsYou => 0xf85149,
        Status::YourTurn => 0xd29922,
        Status::Working => 0x3fb950,
        Status::Unknown => 0x6e7681,
    }
}

fn tint(status: Status) -> u32 {
    match status {
        Status::NeedsYou => 0x33f85149,
        Status::YourTurn => 0x33d29922,
        Status::Working => 0x333fb950,
        Status::Unknown => 0x206e7681,
    }
}

fn row(s: &SessionState, selected: bool) -> impl IntoElement {
    let branch = s.branch.clone().unwrap_or_default();
    div()
        .flex()
        .flex_col()
        .px(px(10.0))
        .py(px(7.0))
        .bg(rgba(tint(s.status)))
        .when(selected, |d| d.border_l_2().border_color(rgb(dot(s.status))))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(div().w(px(8.0)).h(px(8.0)).rounded_full().bg(rgb(dot(s.status))))
                .child(div().text_color(rgb(0xe6edf3)).child(s.project.clone()))
                .child(div().text_color(rgb(0x7d8590)).child(branch)),
        )
        .child(
            div()
                .flex()
                .justify_between()
                .child(div().text_color(rgb(dot(s.status))).child(s.summary.clone()))
                .child(div().text_color(rgb(0x6e7681)).child(format!("{}", s.last_activity))),
        )
}

impl Render for SessionsView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows();
        let selected = self.selected;
        div()
            .id("claude-sessions")
            .track_focus(&self.focus_handle)
            .w(px(330.0))
            .bg(rgb(0x161b22))
            .children(
                rows.iter()
                    .enumerate()
                    .map(|(i, s)| row(s, i == selected))
                    .collect::<Vec<_>>(),
            )
    }
}
```

(The exact gpui method names - `border_l_2`, `rounded_full`, `justify_between` - match those used across `plugin-launcher`/`plugin-alt-tab` render code; verify against `plugin-launcher/src/ui/render.rs` while implementing and adjust to the versions actually in use. The elapsed field renders the raw `last_activity` for now; a `format_elapsed(now - last_activity)` helper can replace it once `now` is threaded in.)

- [ ] **Step 3: Add `pub mod ui;` to `lib.rs`. Build + clippy**

Run: `RUSTFLAGS="-D warnings" cargo clippy -p plugin-claude-sessions --all-targets`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add plugins/plugin-claude-sessions/src/ui plugins/plugin-claude-sessions/src/lib.rs
git commit -m "feat(claude-sessions): GPUI panel view and tinted rows"
```

---

## Task 17: daemon run() - bootstrap, window, threads, keyboard

**Files:**
- Modify: `plugins/plugin-claude-sessions/src/daemon/mod.rs`
- Create: `plugins/plugin-claude-sessions/src/ui/run.rs`
- Modify: `plugins/plugin-claude-sessions/src/main.rs`

Model on `plugin-launcher/src/ui/run.rs`. The daemon process: builds the shared `Arc<Mutex<Registry>>`, starts the hook-ingest listener (Task 12) feeding the registry, starts the action listener (Task 14) feeding a command channel, ensures hooks installed (Task 15), boots the GPUI app, opens the always-on-top popup window with `SessionsView`, runs a reconciler timer, and a `spawn_command_loop` to handle `Open`/`Cleanup`/`Kill`.

- [ ] **Step 1: Implement `src/ui/run.rs`**

```rust
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use gpui::Application;

use crate::daemon::actions::{self, Command};
use crate::daemon::reconcile;
use crate::host::kitty::Kitty;
use crate::host::TerminalHost;
use crate::hooks::ingest::{self, IngestMsg};
use crate::paths;
use crate::registry::{Registry, SessionState};

pub fn run() -> anyhow::Result<()> {
    let registry: Arc<Mutex<Registry>> = Arc::new(Mutex::new(Registry::default()));
    let host: Arc<dyn TerminalHost + Send + Sync> = Arc::new(Kitty);

    reconcile::ensure_hooks_installed();

    let (ing_tx, ing_rx) = mpsc::channel::<IngestMsg>();
    ingest::start_ingest(&paths::hook_socket_path(), ing_tx)?;

    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
    if !actions::start_listener(cmd_tx) {
        anyhow::bail!("action listener failed to bind");
    }

    spawn_ingest_apply(registry.clone(), ing_rx);

    let reg_for_app = registry.clone();
    let host_for_app = host.clone();
    Application::new().run(move |cx| {
        qol_gpui::keepalive::open_keepalive(cx, Some("plugin-claude-sessions"));
        open_panel(reg_for_app.clone(), host_for_app.clone(), cx);
        spawn_reconcile_timer(reg_for_app.clone(), host_for_app.clone(), cx);
        spawn_command_poll(cmd_rx, cx);
    });
    Ok(())
}
```

- [ ] **Step 2: Implement the helpers (ingest apply, panel open, timer, command poll)**

```rust
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn spawn_ingest_apply(registry: Arc<Mutex<Registry>>, rx: mpsc::Receiver<IngestMsg>) {
    std::thread::spawn(move || {
        for msg in rx {
            let Ok(mut reg) = registry.lock() else { continue };
            if msg.remove {
                reg.remove(&msg.session_id);
                continue;
            }
            let project = msg.cwd.rsplit('/').find(|s| !s.is_empty()).unwrap_or(&msg.cwd).to_string();
            reg.upsert(SessionState {
                session_id: msg.session_id,
                pid: msg.pid,
                project,
                cwd: msg.cwd,
                branch: None,
                status: msg.status,
                summary: msg.summary,
                last_activity: now_secs(),
            });
        }
    });
}
```

`open_panel` uses `qol_gpui::window::open_window_with_focus` with `ghost`/popup options (the launcher's `ghost_window_options` shape: `titlebar: None`, `kind: qol_gpui::platform::ghost_window_kind()`, transparent background, `app_id`), then calls `qol_gpui::popup_window::configure_popup_window(title)` to set `NSPopUpMenuWindowLevel`. Compute placement from the configured corner (Task 1 config) against the active monitor bounds (`qol_gpui::monitor`).

`spawn_reconcile_timer` uses `qol_gpui::command_loop` / `cx.spawn` with a `background_executor().timer(Duration::from_secs(poll))` loop that calls `reconcile::tick(&registry, host.as_ref(), now_secs())` then `cx.notify()`s the view (store the view handle in an `Arc<Mutex<Option<WeakEntity<SessionsView>>>>` or refresh by reading the registry in render - simplest: the view reads the registry each render, so the timer only needs to trigger a redraw via the view handle).

`spawn_command_poll` uses `qol_gpui::command_loop::spawn_command_loop(cx, cmd_rx, handler)` where the handler matches:
- `Command::Open` -> show/raise the window (re-open or `activate_window`), focus it.
- `Command::Cleanup` -> `reconcile::remove_hooks()`.
- `Command::Kill` -> `LoopFlow::Stop`.

Implement these against the exact `qol_gpui` signatures captured in the design reference, mirroring `plugin-launcher/src/ui/run.rs:85-115` and `window_host.rs`.

- [ ] **Step 3: Wire `daemon::run()` to `ui::run::run()` and main dispatch**

`src/daemon/mod.rs`:

```rust
pub mod actions;
pub mod reconcile;

pub fn run() -> anyhow::Result<()> {
    crate::ui::run::run()
}
```

`src/main.rs` arms:

```rust
None | Some("daemon") | Some("run") => match plugin_claude_sessions::daemon::run() {
    Ok(()) => ExitCode::SUCCESS,
    Err(e) => { eprintln!("plugin-claude-sessions daemon: {e:#}"); ExitCode::from(1) }
},
Some("open") => { let _ = send_action("open"); ExitCode::SUCCESS }
Some("cleanup") => { let _ = send_action("cleanup"); ExitCode::SUCCESS }
```

Where `send_action` connects the action socket and writes the action (or falls back to `reconcile::remove_hooks()` directly for `cleanup` when the daemon is down). Use `qol_plugin_daemon` `send_action`/the unix_common dispatch shape.

- [ ] **Step 4: Build + clippy + a live smoke run**

Run: `RUSTFLAGS="-D warnings" cargo clippy -p plugin-claude-sessions --all-targets`
Expected: clean.

Live smoke (macOS, kitty running with a `claude` pane):
1. `cargo run -q -p plugin-claude-sessions` (daemon) in one shell.
2. In another: `cargo run -q -p plugin-claude-sessions -- open` -> panel appears, always on top, lists the claude pane.
3. Trigger a Stop in a Claude session -> its row turns yellow within the poll interval.
Expected: rows render and recolor; Enter on a row focuses the kitty window.

- [ ] **Step 5: Commit**

```bash
git add plugins/plugin-claude-sessions/src/daemon/mod.rs plugins/plugin-claude-sessions/src/ui/run.rs plugins/plugin-claude-sessions/src/main.rs plugins/plugin-claude-sessions/src/lib.rs
git commit -m "feat(claude-sessions): daemon bootstrap, panel window, reconcile timer"
```

---

## Task 18: keyboard navigation and Enter-to-focus

**Files:**
- Modify: `plugins/plugin-claude-sessions/src/ui/render.rs`

- [ ] **Step 1: Add key handling to the root element**

On the focused root `div()`, attach `on_key_down` (gpui `KeyDownEvent`) handlers mirroring `plugin-launcher`/`plugin-alt-tab` input code:
- ArrowDown/`j`: `self.selected = (self.selected + 1).min(rows.len().saturating_sub(1)); cx.notify();`
- ArrowUp/`k`: `self.selected = self.selected.saturating_sub(1); cx.notify();`
- Enter: `self.focus_selected();`
- Escape: blur/hide the window (`qol_gpui::popup_window::hide_invisible` or the launcher hide path).
- Digits `1..9`: set `selected` to that index if present, then `focus_selected()`.

```rust
.on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _window, cx| {
    let len = this.rows().len();
    match ev.keystroke.key.as_str() {
        "down" | "j" => { this.selected = (this.selected + 1).min(len.saturating_sub(1)); cx.notify(); }
        "up" | "k" => { this.selected = this.selected.saturating_sub(1); cx.notify(); }
        "enter" => this.focus_selected(),
        "escape" => { /* hide window */ }
        d if d.len() == 1 && d.chars().next().unwrap().is_ascii_digit() => {
            let idx = d.parse::<usize>().unwrap_or(0).saturating_sub(1);
            if idx < len { this.selected = idx; this.focus_selected(); }
        }
        _ => {}
    }
}))
```

- [ ] **Step 2: Build + clippy + manual key test**

Run: `RUSTFLAGS="-D warnings" cargo clippy -p plugin-claude-sessions --all-targets`
Expected: clean. Manual: arrows move the selection border; Enter jumps; Esc hides.

- [ ] **Step 3: Commit**

```bash
git add plugins/plugin-claude-sessions/src/ui/render.rs
git commit -m "feat(claude-sessions): keyboard nav and enter-to-focus"
```

---

## Task 19: manifest structural test and final verification

**Files:**
- Create: `plugins/plugin-claude-sessions/tests/manifest_structural.rs`

- [ ] **Step 1: Write the structural test**

Adapt `86305fa5^:plugins/plugin-claude-sessions/tests/manifest_structural.rs`. Assert against `plugin.toml`:

```rust
use qol_plugin_api::manifest::PluginManifest;

#[test]
fn manifest_declares_daemon_actions_and_one_binary() {
    let toml = include_str!("../plugin.toml");
    let m: PluginManifest = toml::from_str(toml).expect("parse plugin.toml");

    assert_eq!(m.plugin.name, "Claude Sessions");
    assert_eq!(
        m.plugin.platforms.as_deref(),
        Some(["linux".to_string(), "macos".to_string()].as_slice())
    );

    let runtime = m.runtime.expect("runtime");
    assert_eq!(runtime.command, "plugin-claude-sessions");
    let actions = runtime.actions.expect("actions");
    assert!(actions.contains_key("open"));
    assert!(actions.contains_key("cleanup"));

    let daemon = m.daemon.expect("daemon");
    assert!(daemon.enabled);
    assert_eq!(daemon.command, "plugin-claude-sessions");
    assert_eq!(daemon.socket.as_deref(), Some("/tmp/qol-claude-sessions.sock"));

    let bins = m.dependencies.expect("deps").binaries;
    assert_eq!(bins.len(), 1, "single binary keeps store release discovery working");
    assert_eq!(bins[0].name, "plugin-claude-sessions");

    assert!(m.capabilities.gpui);
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p plugin-claude-sessions --test manifest_structural`
Expected: PASS. (Add `toml` to `[dev-dependencies]` if not present.)

- [ ] **Step 3: Full verification gate**

Run, expecting all green:
```bash
cargo fmt -p plugin-claude-sessions
cargo fmt --check -p plugin-claude-sessions
RUSTFLAGS="-D warnings" cargo clippy -p plugin-claude-sessions --all-targets --all-features
cargo test -p plugin-claude-sessions
cargo build -p plugin-claude-sessions --release
```

- [ ] **Step 4: Commit**

```bash
git add plugins/plugin-claude-sessions/tests/manifest_structural.rs plugins/plugin-claude-sessions/Cargo.toml
git commit -m "test(claude-sessions): manifest structural test and final gate"
```

---

## Manual end-to-end validation (after Task 19)

1. Build and install the plugin into a dev qol-tray (or use the in-app Recompile button per `apps/qol-tray/CLAUDE.md`).
2. Bind a host hotkey to the `open` action in qol-tray's hotkey settings.
3. Confirm `~/.claude/settings.json` now contains the managed block (8 events, guarded command with the marker).
4. Open several Claude sessions in kitty panes; press the hotkey -> panel lists them.
5. Drive each color: submit a prompt (green), let a turn finish (yellow), trigger a permission prompt (red).
6. Press Enter on a row -> the kitty window hosting that session is raised.
7. Run the `cleanup` menu action -> the managed block is removed from settings.json; the user's other hooks remain.
8. Delete the plugin binary, fire a Claude event -> no hook error appears in the transcript (the `test -x` guard exits 0).

---

## Self-Review

**Spec coverage:**
- Persistent always-on-top panel: Tasks 16-17 (popup_window + configure_popup_window). Covered.
- Typed status (PermissionRequest/Notification typed): Task 2. Covered.
- Hooks-only discovery + cold host discovery + join by pid: Tasks 7, 10, 15. Covered.
- Self-healing + silent-guarded + manual cleanup: Tasks 5, 15, 17. Covered.
- Two stream sockets (action + hook ingest): Tasks 12, 14. Covered.
- `focus()` + `discover()` host strategy (kitty): Tasks 6, 7, 10. Covered.
- Single binary, argv dispatch: Tasks 1, 13, 17. Covered.
- Two-line tinted rows, sort red>yellow>green: Tasks 4, 16. Covered.
- Toggle via `open` menu action + host hotkey: Tasks 1, 14, 17 + manual step 2. Covered.
- POSIX shell escaping + schema-only marker: Task 5. Covered.
- libproc resolver + encode_cwd + parent_pid lifts: Tasks 3, 8, 9. Covered.
- Cross-platform `-D warnings`: clippy gate in Tasks 8-19. Covered.
- AF_UNIX sun_path short paths: Task 15 (/tmp sockets). Covered.

**Placeholder scan:** Task 14 `reconcile.rs` and Task 17 helpers are described with concrete code plus named qol_gpui calls; the only deliberately deferred detail is the exact gpui builder method spelling, which the implementer verifies against `plugin-launcher` live (flagged inline). No "TBD"/"add error handling"/"similar to Task N" left.

**Type consistency:** `Status` (4 variants incl. `Unknown`) is consistent across status.rs/registry.rs/ingest.rs/ui. `IngestMsg`, `SessionState`, `Pane`, `Registry::{upsert,remove,prune,sorted,contains_pid,get_mut}` names match across tasks. `managed_command`/`install_block`/`remove_block`/`shell_single_quote` are stable. The action socket `CONFIG` field set is flagged for verification against the live `qol-plugin-daemon::DaemonConfig` in Task 14.
