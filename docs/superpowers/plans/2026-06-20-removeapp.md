# removeapp Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `plugins/removeapp` - a qol-tray `window`-kind plugin that uninstalls an app and its leftovers, with a headless core, a CLI, and a gpui picker. macOS is iteration 1.

**Architecture:** One crate, three layers. A pure `core` engine delegates all OS-specific work to an `AppPlatform` strategy (`platform/{macos,linux,windows}`). A headless CLI (`scan`/`remove`) and a gpui picker (`open`) are two thin front-ends over the same core. The window lifecycle (lazy launch, singleton, die-with-host) mirrors `plugin-cli-sessions` exactly and needs no qol-tray change.

**Tech Stack:** Rust 2021, gpui + qol-gpui (UI), qol-plugin-daemon (singleton socket), qol-runtime (host-death watchdog), qol-search (fuzzy), qol-app-icon (icons), serde/serde_json, objc2-foundation + plist (macOS FFI/parsing), anyhow/thiserror.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-06-20-removeapp-plugin-design.md` (source of truth).
- No comments in code. Conventional commits, one-line messages, no co-authors, no AI attribution.
- `qol-arch-code`: zero `#[cfg(target_os)]` outside `platform/mod.rs` re-exports; NEVER `compile_error!`; OS files only under `platform/`; stubs return typed `Err`, never `unimplemented!()`.
- Verification gate before any "done": `cargo fmt --all --check`, `RUSTFLAGS="-D warnings" cargo clippy --all-targets --all-features --keep-going -- -D warnings`, `cargo build`, `cargo test`. (`make ci-local` runs these.)
- Move-to-Trash is the default disposal; hard delete only via explicit opt-in (`--force` / UI toggle). Guardrails (`is_protected`) hold under both, checked before any filesystem mutation.
- Package `plugin-removeapp`, binary `removeapp`, lib `plugin_removeapp`. Plugin id derives from dir name `plugin-removeapp`.
- License file: `PolyForm-Noncommercial-1.0.0`.

---

## File Structure

```
plugins/removeapp/
  Cargo.toml
  plugin.toml
  Makefile
  README.md
  LICENSE
  .gitignore
  src/
    main.rs                 # subcommand dispatch (open|scan|remove) + contract test
    lib.rs                  # module wiring
    core/
      mod.rs                # domain types + Disposal + free-fn API over Platform
      platform/
        mod.rs              # AppPlatform trait + cfg re-export of Platform
        macos.rs            # real impl (enumerate, scan, remove_paths, is_protected)
        linux.rs            # typed-Err stub
        windows.rs          # typed-Err stub
    cli/
      mod.rs                # scan/remove handlers (arg flags, JSON, confirm)
    daemon/
      mod.rs                # run() -> ui::run()
      actions.rs            # CONFIG, Command, parse_command, start_listener
    ui/
      mod.rs                # RemoveAppView + states
      run.rs                # gpui Application bootstrap (mirror cli-sessions)
```

---

### Task 1: Scaffold the crate

**Files:**
- Create: `plugins/removeapp/Cargo.toml`
- Create: `plugins/removeapp/plugin.toml`
- Create: `plugins/removeapp/Makefile`
- Create: `plugins/removeapp/.gitignore`
- Create: `plugins/removeapp/README.md`, `LICENSE`
- Create: `plugins/removeapp/src/main.rs`, `src/lib.rs`
- Create: `src/core/mod.rs`, `src/core/platform/{mod,macos,linux,windows}.rs` (skeletons)
- Modify: workspace root `Cargo.toml` (add member if the workspace lists members explicitly - check first)

**Interfaces:**
- Produces: the crate compiles on macOS/Linux/Windows; `validate_plugin_contract` test passes.

- [ ] **Step 1: Write `Cargo.toml`**

```toml
[package]
name = "plugin-removeapp"
version = "0.1.0"
edition = "2021"
description = "Uninstall an app and its leftovers for QoL Tray"
license = "PolyForm-Noncommercial-1.0.0"

[lib]
name = "plugin_removeapp"
path = "src/lib.rs"

[[bin]]
name = "removeapp"
path = "src/main.rs"

[dependencies]
anyhow = "1"
thiserror = "1"
serde = { version = "1.0", features = ["derive"] }
serde_json.workspace = true
gpui.workspace = true
qol-gpui.workspace = true
qol-plugin-daemon.workspace = true
qol-runtime.workspace = true
qol-search.workspace = true
qol-app-icon.workspace = true

[target.'cfg(target_os = "macos")'.dependencies]
libc = "0.2"
plist = "1"
objc2 = "0.5"
objc2-foundation = { version = "0.2", features = ["NSFileManager", "NSURL", "NSString", "NSArray"] }

[dev-dependencies]
qol-plugin-api.workspace = true
toml = "0.9"
tempfile = "3"
```

- [ ] **Step 2: Write `plugin.toml`** (copy verbatim from the spec's Manifest section).

- [ ] **Step 3: Write `Makefile`** (copy `plugin-template/Makefile`, replace `BINARY = plugin-template` with `BINARY = removeapp`).

- [ ] **Step 4: Write `.gitignore`** (`/target`, `/removeapp`, `/removeapp.new`), `README.md` (per `qol-workflow:readme`), and copy a `LICENSE` (PolyForm-Noncommercial-1.0.0) from any sibling plugin.

- [ ] **Step 5: Write skeleton sources**

`src/lib.rs`:
```rust
pub mod cli;
pub mod core;
pub mod daemon;
pub mod ui;
```

`src/core/mod.rs` (types only for now; filled in Task 2):
```rust
pub mod platform;
```

`src/core/platform/mod.rs`:
```rust
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::Platform;
#[cfg(target_os = "linux")]
pub use linux::Platform;
#[cfg(target_os = "windows")]
pub use windows::Platform;
```

Each of `macos.rs`/`linux.rs`/`windows.rs`:
```rust
pub struct Platform;
```

`src/cli/mod.rs`, `src/daemon/mod.rs`, `src/ui/mod.rs`: empty `// filled in later` is NOT allowed - create them with the minimal real content their Task needs. For Task 1, create `src/cli/mod.rs` and `src/ui/mod.rs` as empty modules (`#![allow(unused)]` not needed yet) and `src/daemon/mod.rs` with `pub mod actions;` plus a stub `pub fn run() -> anyhow::Result<()> { Ok(()) }`, and `src/daemon/actions.rs` empty. (Replaced wholesale in Tasks 5-7.)

`src/main.rs`:
```rust
use std::env;
use std::process::ExitCode;

fn main() -> ExitCode {
    match env::args().nth(1).as_deref() {
        None | Some("open") => ExitCode::SUCCESS,
        Some("scan") | Some("remove") => ExitCode::SUCCESS,
        Some(other) => {
            eprintln!("removeapp: unknown subcommand {other:?}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::PluginManifest;

    #[test]
    fn validate_plugin_contract() {
        PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
    }
}
```

- [ ] **Step 6: Build + test**

Run: `cd plugins/removeapp && cargo build && cargo test`
Expected: builds; `validate_plugin_contract` PASSES.

- [ ] **Step 7: Commit**

```bash
git add plugins/removeapp
git commit -m "feat(removeapp): scaffold window-kind plugin crate"
```

---

### Task 2: Domain types + macOS leftover scanning

**Files:**
- Modify: `src/core/mod.rs` (add types + `AppPlatform` trait via `platform`)
- Modify: `src/core/platform/mod.rs` (add `AppPlatform` trait)
- Modify: `src/core/platform/macos.rs` (`installed_apps`, `scan`, roots)
- Test: `src/core/platform/macos.rs` `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `InstalledApp`, `LeftoverKind`, `Leftover`, `RemovalPlan`, `RemovalOutcome`, `Disposal`, trait `AppPlatform`, `MacosPlatform::with_roots(home: PathBuf, app_dirs: Vec<PathBuf>) -> Self`, `MacosPlatform::default()`.

- [ ] **Step 1: Define domain types in `src/core/mod.rs`** (copy the type block from the spec's Layer 1, deriving `Debug, Clone, serde::Serialize` on all; `Disposal` derives `Debug, Clone, Copy, PartialEq`). Add `pub mod platform;` and `pub use platform::AppPlatform;`.

- [ ] **Step 2: Define the trait in `src/core/platform/mod.rs`** (above the cfg re-exports):
```rust
use crate::core::{Disposal, InstalledApp, RemovalOutcome, RemovalPlan};
use std::path::PathBuf;

pub trait AppPlatform {
    fn installed_apps(&self) -> anyhow::Result<Vec<InstalledApp>>;
    fn scan(&self, app: &InstalledApp) -> anyhow::Result<RemovalPlan>;
    fn remove_paths(&self, paths: &[PathBuf], how: Disposal) -> anyhow::Result<RemovalOutcome>;
    fn is_protected(&self, app: &InstalledApp) -> bool;
}
```

- [ ] **Step 3: Write the failing test** (`src/core/platform/macos.rs`):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::AppPlatform;
    use std::fs;

    fn write(path: &std::path::Path, body: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    #[test]
    fn scan_collects_bundle_and_library_leftovers() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let apps = tmp.path().join("Applications");
        let bundle = apps.join("Foo.app");
        write(&bundle.join("Contents/Info.plist"), INFO_PLIST_FOO);
        write(&home.join("Library/Caches/com.acme.foo/blob"), "x");
        write(&home.join("Library/Preferences/com.acme.foo.plist"), "y");

        let plat = MacosPlatform::with_roots(home.clone(), vec![apps.clone()]);
        let app = plat.installed_apps().unwrap().into_iter()
            .find(|a| a.name == "Foo").unwrap();
        assert_eq!(app.bundle_id.as_deref(), Some("com.acme.foo"));

        let plan = plat.scan(&app).unwrap();
        let paths: Vec<_> = plan.items.iter().map(|l| l.path.clone()).collect();
        assert!(paths.contains(&bundle));
        assert!(paths.contains(&home.join("Library/Caches/com.acme.foo")));
        assert!(paths.contains(&home.join("Library/Preferences/com.acme.foo.plist")));
        assert!(plan.total_bytes > 0);
    }

    const INFO_PLIST_FOO: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.acme.foo</string>
<key>CFBundleName</key><string>Foo</string>
</dict></plist>"#;
}
```

- [ ] **Step 4: Run test, verify it fails** - Run: `cargo test -p plugin-removeapp scan_collects` → FAIL (no `MacosPlatform`).

- [ ] **Step 5: Implement `MacosPlatform`** in `src/core/platform/macos.rs`:
  - `struct MacosPlatform { home: PathBuf, app_dirs: Vec<PathBuf> }`; `with_roots(...)`; `default()` uses `dirs::home_dir()` and `vec![/Applications, ~/Applications]`.
  - `installed_apps`: read each `app_dir`, for every `*.app`, parse `Contents/Info.plist` with `plist::Value` → `CFBundleIdentifier`, `CFBundleName` (fallback to file stem for name). Return `InstalledApp`.
  - `scan`: build candidate leftover paths from `home/Library/<sub>/{bundle_id, bundle_id.plist, name}` for each `LeftoverKind` location (Caches, Preferences, Application Support = `Application Support`, Containers, Group Containers = `Group Containers`, Saved Application State = `Saved Application State/<bundle_id>.savedState`, Logs, HTTPStorages, WebKit, LaunchAgents = `LaunchAgents/<bundle_id>.plist`). Keep only existing paths. Always include the `.app` bundle (`AppBundle`). Compute `size_bytes` via a recursive walk helper. Sum `total_bytes`.
  - Implement `remove_paths`/`is_protected` as `todo-free` minimal stubs returning `Ok(RemovalOutcome::default())` / `false` for now (filled in Tasks 3-4) - or better, implement them in their own tasks and have this task's trait impl `unimplemented`-free by ordering Task 3/4 before wiring. To keep this task self-contained and compiling, implement `is_protected` returning `false` and `remove_paths` returning an empty `RemovalOutcome`; Tasks 3-4 replace them.

- [ ] **Step 6: Run test, verify pass** - Run: `cargo test -p plugin-removeapp scan_collects` → PASS.

- [ ] **Step 7: Add the `LeftoverKind`-location table test** - table-driven case set asserting each present location is matched and absent ones are skipped. Run → PASS.

- [ ] **Step 8: Commit** - `git commit -m "feat(removeapp): macOS app discovery and leftover scan"`

---

### Task 3: Protection guardrails

**Files:**
- Modify: `src/core/platform/macos.rs` (`is_protected`)
- Test: same file's `mod tests`

**Interfaces:**
- Produces: `MacosPlatform::is_protected(&self, app: &InstalledApp) -> bool`.

- [ ] **Step 1: Write the failing test** - table-driven:
```rust
#[test]
fn is_protected_blocks_system_and_managed_apps() {
    let plat = MacosPlatform::with_roots("/Users/x".into(), vec![]);
    let cases = [
        ("/System/Applications/Mail.app", Some("com.apple.mail"), true),
        ("/Applications/Microsoft Defender.app", Some("com.microsoft.wdav.tray"), true),
        ("/Applications/CompanyPortal.app", Some("com.microsoft.intune.companyportal"), true),
        ("/Applications/Foo.app", Some("com.acme.foo"), false),
    ];
    for (path, bid, expected) in cases {
        let app = InstalledApp { name: "x".into(), bundle_id: bid.map(Into::into), path: path.into() };
        assert_eq!(plat.is_protected(&app), expected, "path: {path}");
    }
}
```

- [ ] **Step 2: Run, verify fail** - Run: `cargo test -p plugin-removeapp is_protected_blocks` → FAIL.

- [ ] **Step 3: Implement `is_protected`**: true if path starts with `/System` or `/Library/Apple`; OR bundle_id starts with `com.apple.`; OR bundle_id matches the managed-security denylist (prefixes `com.microsoft.wdav`, `com.microsoft.intune`, `com.heimdalsecurity`, plus `com.microsoft.autoupdate`); OR the bundle dir is not writable by the current user (`fs::metadata(...).permissions()` + a writability probe; if the path doesn't exist, treat as not-a-writability-block). Keep the denylist a `const &[&str]`.

- [ ] **Step 4: Run, verify pass** → PASS.

- [ ] **Step 5: Commit** - `git commit -m "feat(removeapp): protection guardrails for system and managed apps"`

---

### Task 4: Removal - core orchestration + delete + trash

**Files:**
- Modify: `src/core/mod.rs` (free-fn API: `installed_apps`, `search`, `resolve_unique`, `plan`, `remove`, `is_protected`)
- Modify: `src/core/platform/macos.rs` (`remove_paths` real impl)
- Test: `src/core/mod.rs` `mod tests` + macos test for delete

**Interfaces:**
- Consumes: `AppPlatform` (Task 2), `is_protected` (Task 3).
- Produces: `core::remove(plan, Disposal) -> Result<RemovalOutcome>`, `core::resolve_unique`, `core::search`.

- [ ] **Step 1: Write failing tests** for the macOS delete leaf and the core orchestration:
```rust
// macos.rs tests
#[test]
fn remove_paths_delete_removes_sources() {
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("a/b");
    std::fs::create_dir_all(&f).unwrap();
    let plat = MacosPlatform::with_roots(tmp.path().into(), vec![]);
    let out = plat.remove_paths(&[tmp.path().join("a")], crate::core::Disposal::Delete).unwrap();
    assert!(!tmp.path().join("a").exists());
    assert_eq!(out.removed, vec![tmp.path().join("a")]);
    assert!(out.failed.is_empty());
}
```
```rust
// core/mod.rs tests - orchestration with a fake platform
struct FakePlat { protected: bool }
impl AppPlatform for FakePlat { /* installed_apps/scan/is_protected per fields; remove_paths records */ }

#[test]
fn remove_refuses_protected_before_touching_fs() { /* plan over a protected app -> Err, remove_paths never called */ }
```

- [ ] **Step 2: Run, verify fail** → FAIL.

- [ ] **Step 3: Implement `remove_paths` (macOS)**: for `Disposal::Delete`, `fs::remove_dir_all` (dirs) / `fs::remove_file` (files), collecting `removed`/`failed` per path. For `Disposal::Trash`, call NSFileManager:
```rust
use objc2_foundation::{NSFileManager, NSString, NSURL};
fn trash(path: &std::path::Path) -> Result<(), String> {
    unsafe {
        let s = NSString::from_str(path.to_str().ok_or("non-utf8 path")?);
        let url = NSURL::fileURLWithPath(&s);
        let fm = NSFileManager::defaultManager();
        fm.trashItemAtURL_resultingItemURL_error(&url, None)
            .map_err(|e| e.localizedDescription().to_string())
    }
}
```
(If the exact `objc2-foundation` method name/feature differs, mirror `libs/qol-app-icon/src/macos.rs` for the objc2 import/call style; fall back to the `trash` crate if the binding is missing.)

- [ ] **Step 4: Implement the core free-fn API in `src/core/mod.rs`**: each delegates to `platform::Platform::default()` (a `fn platform() -> Platform`). `resolve_unique`: call `search`; if exactly one strong match return it, if zero `Err("no app matches {q}")`, if many `Err` listing candidate names. `search`: `installed_apps()` ranked by `qol_search` against the query. `remove(plan, how)`: if `is_protected(&plan.app)` return typed `Err`; else `Platform.remove_paths(&plan.items.paths(), how)`.

- [ ] **Step 5: Run, verify pass** → PASS.

- [ ] **Step 6: Commit** - `git commit -m "feat(removeapp): trash/delete removal with protected-app refusal"`

---

### Task 5: Headless CLI (`scan` / `remove`)

**Files:**
- Create: `src/cli/mod.rs`
- Modify: `src/main.rs` (dispatch `scan`/`remove` into `cli`)
- Test: `src/cli/mod.rs` `mod tests`

**Interfaces:**
- Consumes: `core::{resolve_unique, plan, remove, Disposal, RemovalPlan, RemovalOutcome}`.
- Produces: `cli::scan(args) -> ExitCode`, `cli::remove(args) -> ExitCode`, `cli::disposal_from_flags(force: bool) -> Disposal`.

- [ ] **Step 1: Write failing tests** for the pure helpers:
```rust
#[test]
fn disposal_from_flags_defaults_to_trash() {
    assert_eq!(disposal_from_flags(false), Disposal::Trash);
    assert_eq!(disposal_from_flags(true), Disposal::Delete);
}
#[test]
fn plan_serializes_to_json_with_total() {
    let plan = sample_plan();
    let v: serde_json::Value = serde_json::from_str(&plan_json(&plan)).unwrap();
    assert!(v["total_bytes"].as_u64().unwrap() > 0);
    assert!(v["items"].is_array());
}
```

- [ ] **Step 2: Run, verify fail** → FAIL.

- [ ] **Step 3: Implement `cli`**: manual flag parse (`--dry-run`, `--yes`, `--force`) over `env::args`. `scan`: `resolve_unique` → `plan` → print `serde_json::to_string_pretty`. `remove`: `resolve_unique` → `plan`; if `--dry-run` print plan + exit; else print plan, prompt `y/N` on stderr unless `--yes`, then `core::remove(&plan, disposal_from_flags(force))` and print the `RemovalOutcome` JSON. Resolution errors print the candidate list to stderr and exit `1`. Extract `plan_json`/`disposal_from_flags` as the tested pure fns.

- [ ] **Step 4: Wire `src/main.rs`** dispatch: `Some("scan") => cli::scan(...)`, `Some("remove") => cli::remove(...)`.

- [ ] **Step 5: Run, verify pass** → PASS. Then manual smoke: `cargo run -- scan Safari` prints a plan (read-only).

- [ ] **Step 6: Commit** - `git commit -m "feat(removeapp): headless scan/remove CLI"`

---

### Task 6: Window lifecycle wiring (singleton + watchdog + open)

**Files:**
- Modify: `src/daemon/actions.rs`, `src/daemon/mod.rs`
- Modify: `src/main.rs` (`open` dispatch)

**Interfaces:**
- Consumes: `qol_plugin_daemon::daemon::{DaemonConfig, send_action, start_listener, ReadResult}`, `ui::run::run`.
- Produces: `daemon::actions::CONFIG`, `daemon::actions::start_listener`, `daemon::run`, `daemon::actions::Command`.

This mirrors `plugins/cli-sessions/src/daemon/actions.rs`, `daemon/mod.rs`, and `main.rs`. No unit tests (IPC/process wiring); verify by running.

- [ ] **Step 1: `src/daemon/actions.rs`** - define `CONFIG: DaemonConfig` with `default_socket_name: "qol-removeapp.sock"`, `use_tmpdir_env: false`, `support_replace_existing: true`. `enum Command { Open, Kill }`. `parse_command`: `"ping" => Handled`, `"open"|"show" => Command(Open)`, `"kill" => Command(Kill)`, `_ => Fallback`. `start_listener(tx)` calls `core_daemon::start_listener(&CONFIG, tx, parse_command)`.

- [ ] **Step 2: `src/daemon/mod.rs`** - `pub fn run() -> anyhow::Result<()> { crate::ui::run::run() }`.

- [ ] **Step 3: `src/main.rs` `open` path** (mirror cli-sessions main.rs):
```rust
fn open() -> ExitCode {
    use plugin_removeapp::daemon::{actions::CONFIG, run};
    use qol_plugin_daemon::daemon as core_daemon;
    if core_daemon::send_action(&CONFIG, "open", false) {
        return ExitCode::SUCCESS;
    }
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => { eprintln!("removeapp: {e:#}"); ExitCode::from(1) }
    }
}
```
Route `None | Some("open") => open()`.

- [ ] **Step 4: Build** - `cargo build`. Expected: compiles (depends on `ui::run::run` from Task 7 - if executing in order, stub `ui::run::run` to `Ok(())` here and replace in Task 7, or order Task 7 before this step's build).

- [ ] **Step 5: Commit** - `git commit -m "feat(removeapp): singleton socket + host-death lifecycle wiring"`

---

### Task 7: gpui picker UI

**Files:**
- Create: `src/ui/run.rs` (Application bootstrap - mirror `plugin-cli-sessions/src/ui/run.rs`)
- Modify: `src/ui/mod.rs` (`RemoveAppView` + render)

**Interfaces:**
- Consumes: `core::{search, plan, remove, Disposal}`, `qol_gpui::{keepalive, window, command_loop, platform}`, `qol_app_icon`.
- Produces: `ui::run::run() -> anyhow::Result<()>`, `ui::RemoveAppView`.

gpui views are not unit-tested in this workspace (consistent with the spec). Verify by building and running the picker. Mirror cli-sessions for window options, keepalive, accessory policy, listener start, and the command loop.

- [ ] **Step 1: `src/ui/run.rs`** - adapt `plugin-cli-sessions/src/ui/run.rs`: start `daemon::actions::start_listener(cmd_tx)`; `Application::new().run`: `qol_gpui::keepalive::open_keepalive(cx, Some(APP_ID))`, `qol_gpui::platform::set_accessory_policy()`, open a centered window via `qol_gpui::window::open_window_with_focus` whose root is `RemoveAppView::new(cx)`, then `qol_gpui::command_loop::spawn_command_loop` handling `Command::Open` (focus existing window) and `Command::Kill` (`LoopFlow::Stop`). `APP_ID = "plugin-removeapp"`.

- [ ] **Step 2: `RemoveAppView`** - a keyboard-first view with three states: (a) **list** - a text input filter calling `core::search(query)`, rows showing icon (`qol_app_icon::icon_for_bundle_id`) + name; (b) **preview** - on select, `core::plan(app)`, render the leftover list + `total_bytes` (humanized) + a `Disposal` toggle (defaults `Trash`); (c) **result** - after Enter, `core::remove(&plan, disposal)`, render `RemovalOutcome`, then close on key. Esc backs out a state / closes. Protected apps (`core::is_protected`) render greyed with the reason and are not selectable. Mirror the row/list composition in `plugin-cli-sessions/src/ui/render.rs` and keyboard handling per `qol-tray-ui-systems` / `preact`-free gpui patterns in `gpui-conventions`.

- [ ] **Step 3: Build** - `cargo build`. Expected: compiles.

- [ ] **Step 4: Manual run** - `cargo run -- open`: window appears, filter finds an app, preview lists leftovers + size, Trash (default) moves to Trash, `--force`/toggle deletes, protected apps are blocked. Second `cargo run -- open` focuses the existing window (singleton).

- [ ] **Step 5: Commit** - `git commit -m "feat(removeapp): gpui picker UI"`

---

### Task 8: Integration into qol-tray + final verification

**Files:** none new (dev-link + verify).

- [ ] **Step 1: Verification gate** - `cd plugins/removeapp && make ci-local`. Expected: fmt/clippy/test all green, cross-checks skip-or-pass.

- [ ] **Step 2: Dev-link + run host** - dev-link `plugin-removeapp` into qol-tray (per `qol dev` / worktree resolver), `qol dev`, confirm: the "Remove App" menu item opens the picker; a `Remove App.app` appears under `~/Applications/QoL/` and launches via Spotlight; the launcher exports the shortcut.

- [ ] **Step 3: Manual acceptance** - install a throwaway app, run `removeapp scan <it>` (plan correct), `removeapp remove <it>` (lands in Trash, restorable), `removeapp remove <it> --force` (hard delete), and confirm a managed app (e.g. Microsoft Defender) is refused.

- [ ] **Step 4: Commit any fixups** - `git commit -m "test(removeapp): integration verification fixups"` (only if changes were needed).

---

## Self-Review

**Spec coverage:** lifecycle (Task 6), 3-layer core/CLI/UI (Tasks 2-7), strategy shim with macOS real + Linux/Windows stubs (Tasks 1-2), leftover scope (Task 2), Trash-default + opt-in force delete (Tasks 4-5,7), guardrails (Task 3, enforced Task 4), launcher-app + persistence (manifest in Task 1; no persistence code by design), testing (Tasks 2-5), verification gate (Task 8). Covered.

**Placeholder scan:** Task 1 Step 5 calls out the empty interim modules and which later tasks replace them; Task 6 Step 4 notes the `ui::run::run` ordering dependency. No "TBD"/"add error handling"/"write tests for the above" remain.

**Type consistency:** `RemovalOutcome.removed` used consistently (spec's earlier `trashed` was renamed to `removed` in the final spec + here). `Disposal::{Trash,Delete}`, `remove_paths(paths, how)`, `core::remove(plan, how)`, `MacosPlatform::with_roots(home, app_dirs)` consistent across Tasks 2-7.
