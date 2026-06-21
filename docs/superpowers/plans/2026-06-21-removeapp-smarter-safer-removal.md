# removeapp Smarter, Safer Removal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** `docs/superpowers/specs/2026-06-21-removeapp-smarter-safer-removal-design.md`

**Goal:** Upgrade `plugin-removeapp`'s per-app removal to find owner-related leftovers (without sweeping a sibling's data), detect running and Homebrew-managed apps, and enforce one consent-gated guard contract in both the GPUI picker and the terminal CLI.

**Architecture:** Pure index/disposal/cask logic lives in `core/` (`classify.rs`, `guards.rs`) and is table-tested with zero filesystem. All OS behavior (directory enumeration, `NSWorkspace`, `brew`, `mdfind`, canonicalization) stays behind the `AppPlatform` trait in `core/platform/macos.rs`, with typed stubs on linux/windows. The inventory of installed apps is discovered once and threaded into `scan`/`cask_status`/`guards`.

**Tech Stack:** Rust, `anyhow`, `serde`/`serde_json`, `plist`, `trash`, `dirs`, `libc`, `objc2`/`objc2-app-kit`/`objc2-foundation` (macOS guard FFI), `wait-timeout` (subprocess timeout), gpui + qol-gpui (picker).

## Global Constraints

- No comments in code. Conventional commits, one-line messages, no co-authors, no AI attribution.
- Atomic commits: each task's final commit compiles and passes `cargo test -p plugin-removeapp`.
- Build/test gate before every commit: `cargo test -p plugin-removeapp`, `cargo fmt -p plugin-removeapp -- --check`, `cargo clippy -p plugin-removeapp --all-targets -- -D warnings`. Real output, never assumed.
- Platform code stays behind `AppPlatform` + `platform/{macos,linux,windows}.rs`; no `#[cfg(target_os)]` in business logic; no `compile_error!`; stubs return typed `Err`/safe defaults (qol-arch-code).
- Trash is the default. Hard delete is opt-in via `--force` and applies only to the app bundle + **exact** leftovers; **fuzzy leftovers are always Trash, even under `--force`**. An `Unavailable` cask downgrades even the bundle to Trash under `--force`.
- `--yes` confirms the ordinary prompt only; it never waives the running-app or Homebrew guards.
- Protected apps are refused before any filesystem mutation (unchanged).
- Newtypes for domain concepts; make invalid states unrepresentable; exhaustive matching (no `_ =>` over owned enums).

---

## File Structure

- `core/mod.rs` (modify) - domain types; adds `MatchKind` field to `Leftover`, `freed_bytes` to `RemovalOutcome`, `snapshots` to `RemovalPlan`; rewires free fns to thread inventory + per-item disposal + two-phase removal.
- `core/classify.rs` (create) - pure: `normalize_entry`, `belongs_to`, `owner_of`, `MatchKind`, `effective_disposal`.
- `core/guards.rs` (create) - pure: `CaskToken`, `CaskStatus`, `Guards`, `BasenameOwner`, `parse_cask_map`, `cask_status_for`, `sanitize_stderr`.
- `core/platform/mod.rs` (modify) - `AppPlatform` trait: inventory params, `remove_items`, guard methods.
- `core/platform/macos.rs` (modify) - enumerate+classify scan, canonical paths, `mdfind`, `remove_items`, `is_running`/`quit` (objc2), `cask_status`/`brew_uninstall` (brew subprocess), `IdentitySnapshot`.
- `core/platform/linux.rs`, `core/platform/windows.rs` (modify) - stub updates for new trait shape.
- `cli/mod.rs` (modify) - `--quit`/`--brew`/`--trash-anyway` flags, guard contract, JSON output contract.
- `ui/mod.rs` (modify) - guard banner, guard state machine, freed-bytes readout.
- `Cargo.toml` (modify) - add `objc2-app-kit`, `objc2-foundation`, `wait-timeout` (macOS target).

---

## Task 1: Pure matching rule and MatchKind

**Files:**
- Create: `plugins/plugin-removeapp/src/core/classify.rs`
- Modify: `plugins/plugin-removeapp/src/core/mod.rs` (add `pub mod classify;` + re-export)

**Interfaces:**
- Produces: `pub enum MatchKind { Exact, Fuzzy }` (Copy, Eq, Serialize); `pub fn normalize_entry(entry: &str) -> &str`; `pub fn belongs_to(entry: &str, bid: &str) -> bool`; `pub fn owner_of<'a>(entry: &str, bids: &'a [String]) -> Option<&'a str>`.

- [ ] **Step 1: Write the failing test**

Create `plugins/plugin-removeapp/src/core/classify.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn belongs_to_matches_exact_and_dot_boundary_only() {
        let cases = [
            ("com.acme.foo", "com.acme.foo", true),
            ("com.acme.foo.helper", "com.acme.foo", true),
            ("com.acme.foo.plist", "com.acme.foo", true),
            ("com.acme.foo.savedState", "com.acme.foo", true),
            ("com.acme.foobar", "com.acme.foo", false),
            ("com.acme.fo", "com.acme.foo", false),
        ];
        for (entry, bid, expected) in cases {
            assert_eq!(belongs_to(entry, bid), expected, "entry={entry} bid={bid}");
        }
    }

    #[test]
    fn owner_of_picks_longest_matching_bundle_id() {
        let bids = vec!["com.acme.foo".to_string(), "com.acme.foo.bar".to_string()];
        let cases = [
            ("com.acme.foo.helper", Some("com.acme.foo")),
            ("com.acme.foo.bar.cache", Some("com.acme.foo.bar")),
            ("com.acme.foobar", None),
        ];
        for (entry, expected) in cases {
            assert_eq!(owner_of(entry, &bids), expected, "entry={entry}");
        }
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p plugin-removeapp classify`
Expected: FAIL - `cannot find function belongs_to` (module not yet wired / fns missing).

- [ ] **Step 3: Write minimal implementation**

Prepend to `plugins/plugin-removeapp/src/core/classify.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum MatchKind {
    Exact,
    Fuzzy,
}

pub fn normalize_entry(entry: &str) -> &str {
    entry
        .strip_suffix(".plist")
        .or_else(|| entry.strip_suffix(".savedState"))
        .unwrap_or(entry)
}

pub fn belongs_to(entry: &str, bid: &str) -> bool {
    let e = normalize_entry(entry);
    e == bid || e.starts_with(&format!("{bid}."))
}

pub fn owner_of<'a>(entry: &str, bids: &'a [String]) -> Option<&'a str> {
    bids.iter()
        .filter(|b| belongs_to(entry, b))
        .max_by_key(|b| b.len())
        .map(String::as_str)
}
```

In `plugins/plugin-removeapp/src/core/mod.rs`, add near the top (after `pub mod platform;`):

```rust
pub mod classify;
pub use classify::MatchKind;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p plugin-removeapp classify`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add plugins/plugin-removeapp/src/core/classify.rs plugins/plugin-removeapp/src/core/mod.rs
git commit -m "feat(removeapp): pure owner-aware leftover matching rule"
```

---

## Task 2: Leftover provenance and effective disposal

**Files:**
- Modify: `plugins/plugin-removeapp/src/core/mod.rs` (add `match_kind` to `Leftover`, `freed_bytes` to `RemovalOutcome`)
- Modify: `plugins/plugin-removeapp/src/core/classify.rs` (add `effective_disposal`)
- Modify: `plugins/plugin-removeapp/src/core/platform/macos.rs`, `src/cli/mod.rs` (fix `Leftover { .. }` literals to set `match_kind`)

**Interfaces:**
- Consumes: `MatchKind`, `Disposal` (existing), `LeftoverKind::AppBundle`.
- Produces: `Leftover { path, kind, size_bytes, match_kind }`; `RemovalOutcome { removed, failed, freed_bytes }`; `pub fn effective_disposal(match_kind: MatchKind, requested: Disposal, bundle_trash_override: bool) -> Disposal`.

- [ ] **Step 1: Write the failing test**

Add to `plugins/plugin-removeapp/src/core/classify.rs` `tests` module:

```rust
    #[test]
    fn effective_disposal_keeps_fuzzy_in_trash_always() {
        use crate::core::Disposal;
        let cases = [
            (MatchKind::Exact, Disposal::Delete, false, Disposal::Delete),
            (MatchKind::Exact, Disposal::Trash, false, Disposal::Trash),
            (MatchKind::Exact, Disposal::Delete, true, Disposal::Trash),
            (MatchKind::Fuzzy, Disposal::Delete, false, Disposal::Trash),
            (MatchKind::Fuzzy, Disposal::Delete, true, Disposal::Trash),
        ];
        for (mk, req, override_trash, expected) in cases {
            assert_eq!(
                effective_disposal(mk, req, override_trash),
                expected,
                "mk={mk:?} req={req:?} override={override_trash}"
            );
        }
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p plugin-removeapp effective_disposal`
Expected: FAIL - `cannot find function effective_disposal`.

- [ ] **Step 3: Write minimal implementation**

Add to `core/classify.rs` (after `owner_of`):

```rust
use crate::core::Disposal;

pub fn effective_disposal(
    match_kind: MatchKind,
    requested: Disposal,
    bundle_trash_override: bool,
) -> Disposal {
    match (match_kind, bundle_trash_override) {
        (MatchKind::Fuzzy, _) => Disposal::Trash,
        (MatchKind::Exact, true) => Disposal::Trash,
        (MatchKind::Exact, false) => requested,
    }
}
```

In `core/mod.rs`, extend `Leftover` and `RemovalOutcome`:

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct Leftover {
    pub path: PathBuf,
    pub kind: LeftoverKind,
    pub size_bytes: u64,
    pub match_kind: MatchKind,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RemovalOutcome {
    pub removed: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
    pub freed_bytes: u64,
}
```

Fix every existing `Leftover { .. }` literal to add `match_kind`:
- `core/platform/macos.rs`: the app-bundle item -> `match_kind: MatchKind::Exact`; leftover-candidate items -> set per classification (Task 5 refines; for now `MatchKind::Exact`). Add `use crate::core::MatchKind;`.
- `cli/mod.rs` test `sample_plan()` -> `match_kind: MatchKind::Exact`.
- `core/mod.rs` test `plan_for()` -> `match_kind: MatchKind::Exact`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p plugin-removeapp`
Expected: PASS (all existing tests + `effective_disposal_*`). Fix any remaining `Leftover` literal compile errors the same way.

- [ ] **Step 5: Commit**

```bash
git add plugins/plugin-removeapp/src/core
git commit -m "feat(removeapp): per-item match provenance and effective disposal"
```

---

## Task 3: Guard types, cask map parser, stderr sanitizer (pure)

**Files:**
- Create: `plugins/plugin-removeapp/src/core/guards.rs`
- Modify: `plugins/plugin-removeapp/src/core/mod.rs` (`pub mod guards;` + re-exports)

**Interfaces:**
- Produces: `pub struct CaskToken(String)` with `pub fn parse(&str) -> Option<CaskToken>` + `pub fn as_str(&self) -> &str`; `pub enum CaskStatus { Managed(CaskToken), NotManaged, Unavailable(String) }`; `pub struct Guards { pub running: bool, pub cask: CaskStatus }`; `pub enum BasenameOwner { One(CaskToken), Many }`; `pub fn parse_cask_map(json: &str) -> anyhow::Result<std::collections::BTreeMap<String, BasenameOwner>>`; `pub fn cask_status_for(target_basename: &str, map: &BTreeMap<String, BasenameOwner>, inventory_basenames: &[String]) -> CaskStatus`; `pub fn sanitize_stderr(raw: &[u8], cap: usize) -> String`.

- [ ] **Step 1: Write the failing test**

Create `plugins/plugin-removeapp/src/core/guards.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"{"casks":[
      {"token":"discord","artifacts":[{"app":["Discord.app"]}]},
      {"token":"hiddenbar","artifacts":[{"app":["Hidden Bar.app"]}]},
      {"token":"vscode","artifacts":[{"app":[["Code - Insiders.app",{"target":"Code.app"}]]}]},
      {"token":"font-x","artifacts":[{"font":["X.otf"]}]},
      {"token":"dup-a","artifacts":[{"app":["Same.app"]}]},
      {"token":"dup-b","artifacts":[{"app":["Same.app"]}]}
    ]}"#;

    #[test]
    fn cask_token_parse_rejects_illegal_and_leading_dash() {
        assert!(CaskToken::parse("google-chrome").is_some());
        assert!(CaskToken::parse("font-jetbrains-mono").is_some());
        assert!(CaskToken::parse("-evil").is_none());
        assert!(CaskToken::parse("a b").is_none());
        assert!(CaskToken::parse("").is_none());
    }

    #[test]
    fn cask_map_resolves_target_skips_non_app_and_marks_collisions() {
        let map = parse_cask_map(FIXTURE).unwrap();
        assert!(matches!(map.get("Discord.app"), Some(BasenameOwner::One(_))));
        assert!(matches!(map.get("Code.app"), Some(BasenameOwner::One(_))), "target: resolved");
        assert!(map.get("Code - Insiders.app").is_none(), "source name not keyed when target present");
        assert!(map.get("X.otf").is_none(), "non-app artifact skipped");
        assert!(matches!(map.get("Same.app"), Some(BasenameOwner::Many)), "two casks collide");
    }

    #[test]
    fn cask_status_classifies_managed_notmanaged_unavailable() {
        let map = parse_cask_map(FIXTURE).unwrap();
        let one = vec!["Discord.app".to_string()];
        let two = vec!["Discord.app".to_string(), "Discord.app".to_string()];
        assert!(matches!(cask_status_for("Discord.app", &map, &one), CaskStatus::Managed(_)));
        assert!(matches!(cask_status_for("Firefox.app", &map, &one), CaskStatus::NotManaged));
        assert!(matches!(cask_status_for("Same.app", &map, &one), CaskStatus::Unavailable(_)));
        assert!(matches!(cask_status_for("Discord.app", &map, &two), CaskStatus::Unavailable(_)));
        assert!(parse_cask_map("not json").is_err());
    }

    #[test]
    fn sanitize_stderr_strips_control_and_caps() {
        let raw = b"\x1b[31merror\x1b[0m\x07 happened";
        let out = sanitize_stderr(raw, 64);
        assert_eq!(out, "error happened");
        assert_eq!(sanitize_stderr(b"abcdef", 3), "abc");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p plugin-removeapp guards`
Expected: FAIL - types/functions not found.

- [ ] **Step 3: Write minimal implementation**

Prepend to `core/guards.rs`:

```rust
use std::collections::BTreeMap;

use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaskToken(String);

impl CaskToken {
    pub fn parse(s: &str) -> Option<CaskToken> {
        let ok = !s.is_empty()
            && s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
            && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '_' | '-'));
        ok.then(|| CaskToken(s.to_string()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug)]
pub enum CaskStatus {
    Managed(CaskToken),
    NotManaged,
    Unavailable(String),
}

#[derive(Debug)]
pub struct Guards {
    pub running: bool,
    pub cask: CaskStatus,
}

#[derive(Debug)]
pub enum BasenameOwner {
    One(CaskToken),
    Many,
}

#[derive(Deserialize)]
struct BrewInfo {
    casks: Vec<BrewCask>,
}

#[derive(Deserialize)]
struct BrewCask {
    token: String,
    #[serde(default)]
    artifacts: Vec<serde_json::Value>,
}

fn app_basenames(cask: &BrewCask) -> Vec<String> {
    let mut out = Vec::new();
    for artifact in &cask.artifacts {
        let Some(apps) = artifact.get("app").and_then(|a| a.as_array()) else {
            continue;
        };
        let targets: Vec<String> = apps
            .iter()
            .filter_map(|e| e.as_object()?.get("target")?.as_str().map(str::to_string))
            .collect();
        if targets.is_empty() {
            out.extend(apps.iter().filter_map(|e| e.as_str().map(str::to_string)));
        } else {
            out.extend(targets);
        }
    }
    out
}

pub fn parse_cask_map(json: &str) -> Result<BTreeMap<String, BasenameOwner>> {
    let info: BrewInfo = serde_json::from_str(json)?;
    let mut map: BTreeMap<String, BasenameOwner> = BTreeMap::new();
    for cask in &info.casks {
        let Some(token) = CaskToken::parse(&cask.token) else {
            continue;
        };
        for base in app_basenames(cask) {
            match map.get(&base) {
                None => {
                    map.insert(base, BasenameOwner::One(token.clone()));
                }
                Some(_) => {
                    map.insert(base, BasenameOwner::Many);
                }
            }
        }
    }
    Ok(map)
}

pub fn cask_status_for(
    target_basename: &str,
    map: &BTreeMap<String, BasenameOwner>,
    inventory_basenames: &[String],
) -> CaskStatus {
    match map.get(target_basename) {
        None => CaskStatus::NotManaged,
        Some(BasenameOwner::Many) => {
            CaskStatus::Unavailable(format!("{target_basename}: multiple casks share this name"))
        }
        Some(BasenameOwner::One(token)) => {
            let shared = inventory_basenames
                .iter()
                .filter(|b| b.as_str() == target_basename)
                .count();
            if shared > 1 {
                CaskStatus::Unavailable(format!("{target_basename}: {shared} installed apps share this name"))
            } else {
                CaskStatus::Managed(token.clone())
            }
        }
    }
}

impl Clone for CaskToken {
    fn clone(&self) -> Self {
        CaskToken(self.0.clone())
    }
}

pub fn sanitize_stderr(raw: &[u8], cap: usize) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            while let Some(&n) = chars.peek() {
                chars.next();
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        if c == '\n' || c == '\t' || !c.is_control() {
            out.push(c);
        }
    }
    out.truncate(cap);
    out.trim().to_string()
}
```

Add to `core/mod.rs`:

```rust
pub mod guards;
pub use guards::{CaskStatus, CaskToken, Guards};
```

Note: remove the manual `impl Clone for CaskToken` and instead `#[derive(Clone)]` on the struct (kept explicit above only to show intent; prefer the derive). Final form: `#[derive(Debug, Clone, PartialEq, Eq)] pub struct CaskToken(String);` and delete the hand-written impl.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p plugin-removeapp guards`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add plugins/plugin-removeapp/src/core/guards.rs plugins/plugin-removeapp/src/core/mod.rs
git commit -m "feat(removeapp): cask token, tri-state status, and brew json parser"
```

---

## Task 4: Trait reshape, inventory threading, two-phase removal, stubs

**Files:**
- Modify: `plugins/plugin-removeapp/src/core/platform/mod.rs` (trait)
- Modify: `plugins/plugin-removeapp/src/core/platform/linux.rs`, `windows.rs` (stubs)
- Modify: `plugins/plugin-removeapp/src/core/platform/macos.rs` (adapt signatures + `remove_items`)
- Modify: `plugins/plugin-removeapp/src/core/mod.rs` (free fns: `plan`, `remove`, `guards`, `resolve_unique`)

**Interfaces:**
- Produces (trait):
  ```rust
  fn installed_apps(&self) -> Result<Vec<InstalledApp>>;
  fn scan(&self, app: &InstalledApp, inventory: &[InstalledApp]) -> Result<RemovalPlan>;
  fn remove_items(&self, items: &[(PathBuf, Disposal)]) -> Result<RemovalOutcome>;
  fn is_protected(&self, app: &InstalledApp) -> bool;
  fn is_running(&self, app: &InstalledApp) -> bool;
  fn quit(&self, app: &InstalledApp) -> Result<()>;
  fn cask_status(&self, app: &InstalledApp, inventory: &[InstalledApp]) -> CaskStatus;
  fn brew_uninstall(&self, token: &CaskToken) -> Result<()>;
  ```
- Produces (core free fns): `pub fn plan(app, inventory)`, `pub fn remove(plan, requested, cask) -> Result<RemovalOutcome>`, `pub fn guards(app, inventory) -> Guards`, `pub fn resolve_unique(inventory, query)`.

- [ ] **Step 1: Write the failing test**

Replace the FakePlat tests in `core/mod.rs` with the new shape and add a two-phase test:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::guards::CaskStatus;
    use std::cell::RefCell;

    fn app(name: &str, bid: &str) -> InstalledApp {
        InstalledApp { name: name.into(), bundle_id: Some(bid.into()), path: PathBuf::from(format!("/Applications/{name}.app")) }
    }

    fn leftover(path: &str, kind: LeftoverKind, mk: MatchKind) -> Leftover {
        Leftover { path: PathBuf::from(path), kind, size_bytes: 10, match_kind: mk }
    }

    #[derive(Default)]
    struct FakePlat {
        fail_bundle: bool,
        removed: RefCell<Vec<(PathBuf, Disposal)>>,
    }

    impl AppPlatform for FakePlat {
        fn installed_apps(&self) -> Result<Vec<InstalledApp>> { Ok(vec![]) }
        fn scan(&self, app: &InstalledApp, _inv: &[InstalledApp]) -> Result<RemovalPlan> {
            Ok(RemovalPlan {
                items: vec![leftover(app.path.to_str().unwrap(), LeftoverKind::AppBundle, MatchKind::Exact)],
                app: app.clone(), total_bytes: 10, snapshots: vec![],
            })
        }
        fn remove_items(&self, items: &[(PathBuf, Disposal)]) -> Result<RemovalOutcome> {
            let mut out = RemovalOutcome::default();
            for (p, d) in items {
                self.removed.borrow_mut().push((p.clone(), *d));
                if self.fail_bundle && p.to_string_lossy().ends_with(".app") {
                    out.failed.push((p.clone(), "boom".into()));
                } else {
                    out.removed.push(p.clone());
                    out.freed_bytes += 10;
                }
            }
            Ok(out)
        }
        fn is_protected(&self, _app: &InstalledApp) -> bool { false }
        fn is_running(&self, _app: &InstalledApp) -> bool { false }
        fn quit(&self, _app: &InstalledApp) -> Result<()> { Ok(()) }
        fn cask_status(&self, _app: &InstalledApp, _inv: &[InstalledApp]) -> CaskStatus { CaskStatus::NotManaged }
        fn brew_uninstall(&self, _token: &CaskToken) -> Result<()> { Ok(()) }
    }

    fn plan_with(app: InstalledApp, leftovers: Vec<Leftover>) -> RemovalPlan {
        let mut items = vec![leftover(app.path.to_str().unwrap(), LeftoverKind::AppBundle, MatchKind::Exact)];
        items.extend(leftovers);
        RemovalPlan { app, items, total_bytes: 0, snapshots: vec![] }
    }

    #[test]
    fn two_phase_aborts_leftovers_when_bundle_removal_fails() {
        let fake = FakePlat { fail_bundle: true, ..Default::default() };
        let p = plan_with(app("Foo", "com.acme.foo"), vec![leftover("/x/cache", LeftoverKind::Caches, MatchKind::Exact)]);
        let out = remove_with(&fake, &p, Disposal::Trash, &CaskStatus::NotManaged).unwrap();
        assert_eq!(out.removed.len(), 0, "nothing removed");
        assert_eq!(fake.removed.borrow().len(), 1, "only the bundle was attempted, leftovers untouched");
    }

    #[test]
    fn fuzzy_leftover_is_trashed_even_when_delete_requested() {
        let fake = FakePlat::default();
        let p = plan_with(app("Foo", "com.acme.foo"), vec![leftover("/x/fuzzy", LeftoverKind::Caches, MatchKind::Fuzzy)]);
        remove_with(&fake, &p, Disposal::Delete, &CaskStatus::NotManaged).unwrap();
        let recorded = fake.removed.borrow();
        let fuzzy = recorded.iter().find(|(p, _)| p.ends_with("fuzzy")).unwrap();
        assert_eq!(fuzzy.1, Disposal::Trash, "fuzzy forced to Trash");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p plugin-removeapp`
Expected: FAIL - compile errors (trait shape, `snapshots` field, `remove_with` signature).

- [ ] **Step 3: Write minimal implementation**

Update `core/platform/mod.rs`:

```rust
use std::path::PathBuf;

use crate::core::guards::{CaskStatus, CaskToken};
use crate::core::{Disposal, InstalledApp, RemovalOutcome, RemovalPlan};

pub trait AppPlatform {
    fn installed_apps(&self) -> anyhow::Result<Vec<InstalledApp>>;
    fn scan(&self, app: &InstalledApp, inventory: &[InstalledApp]) -> anyhow::Result<RemovalPlan>;
    fn remove_items(&self, items: &[(PathBuf, Disposal)]) -> anyhow::Result<RemovalOutcome>;
    fn is_protected(&self, app: &InstalledApp) -> bool;
    fn is_running(&self, app: &InstalledApp) -> bool;
    fn quit(&self, app: &InstalledApp) -> anyhow::Result<()>;
    fn cask_status(&self, app: &InstalledApp, inventory: &[InstalledApp]) -> CaskStatus;
    fn brew_uninstall(&self, token: &CaskToken) -> anyhow::Result<()>;
}
```

Add `snapshots: Vec<IdentitySnapshot>` to `RemovalPlan` in `core/mod.rs` (define a placeholder `IdentitySnapshot` now; Task 7 fills it in):

```rust
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct IdentitySnapshot;

#[derive(Debug, Clone, serde::Serialize)]
pub struct RemovalPlan {
    pub app: InstalledApp,
    pub items: Vec<Leftover>,
    pub total_bytes: u64,
    #[serde(skip)]
    pub snapshots: Vec<IdentitySnapshot>,
}
```

Rewrite the free fns in `core/mod.rs`:

```rust
pub fn installed_apps() -> Result<Vec<InstalledApp>> {
    platform().installed_apps()
}

pub fn plan(app: &InstalledApp, inventory: &[InstalledApp]) -> Result<RemovalPlan> {
    platform().scan(app, inventory)
}

pub fn guards(app: &InstalledApp, inventory: &[InstalledApp]) -> Guards {
    let plat = platform();
    Guards { running: plat.is_running(app), cask: plat.cask_status(app, inventory) }
}

pub fn resolve_unique(inventory: &[InstalledApp], query: &str) -> Result<InstalledApp> {
    pick_unique(inventory.to_vec(), query)
}

pub fn remove(plan: &RemovalPlan, requested: Disposal, cask: &CaskStatus) -> Result<RemovalOutcome> {
    remove_with(&platform(), plan, requested, cask)
}

fn remove_with(
    plat: &impl AppPlatform,
    plan: &RemovalPlan,
    requested: Disposal,
    cask: &CaskStatus,
) -> Result<RemovalOutcome> {
    if plat.is_protected(&plan.app) {
        anyhow::bail!("removeapp: {} is protected and cannot be removed", plan.app.name);
    }
    let cask_unavailable = matches!(cask, CaskStatus::Unavailable(_));
    let disposal_for = |item: &Leftover| {
        let bundle_override = item.kind == LeftoverKind::AppBundle && cask_unavailable;
        classify::effective_disposal(item.match_kind, requested, bundle_override)
    };

    let (bundle, rest): (Vec<&Leftover>, Vec<&Leftover>) = plan
        .items
        .iter()
        .partition(|i| i.kind == LeftoverKind::AppBundle);

    let mut outcome = RemovalOutcome::default();
    for item in &bundle {
        let res = plat.remove_items(&[(item.path.clone(), disposal_for(item))])?;
        merge(&mut outcome, res, item.size_bytes);
        if !outcome.failed.is_empty() {
            return Ok(outcome);
        }
    }
    let rest_items: Vec<(PathBuf, Disposal)> =
        rest.iter().map(|i| (i.path.clone(), disposal_for(i))).collect();
    let res = plat.remove_items(&rest_items)?;
    let sizes: u64 = rest.iter().map(|i| i.size_bytes).sum();
    merge(&mut outcome, res, sizes);
    Ok(outcome)
}

fn merge(acc: &mut RemovalOutcome, res: RemovalOutcome, _attempted: u64) {
    acc.removed.extend(res.removed);
    acc.failed.extend(res.failed);
    acc.freed_bytes += res.freed_bytes;
}
```

Add imports at the top of `core/mod.rs`: `use crate::core::guards::{CaskStatus, CaskToken, Guards};` and `use classify;` already via `pub mod classify;`. Ensure `LeftoverKind` derives `PartialEq` for the `== AppBundle` comparisons.

Update `core/platform/linux.rs` (and mirror in `windows.rs`):

```rust
use std::path::PathBuf;

use anyhow::{anyhow, Result};

use crate::core::guards::{CaskStatus, CaskToken};
use crate::core::{AppPlatform, Disposal, InstalledApp, RemovalOutcome, RemovalPlan};

#[derive(Default)]
pub struct Platform;

const UNSUPPORTED: &str = "removeapp: not implemented on this platform yet";

impl AppPlatform for Platform {
    fn installed_apps(&self) -> Result<Vec<InstalledApp>> { Err(anyhow!(UNSUPPORTED)) }
    fn scan(&self, _app: &InstalledApp, _inv: &[InstalledApp]) -> Result<RemovalPlan> { Err(anyhow!(UNSUPPORTED)) }
    fn remove_items(&self, _items: &[(PathBuf, Disposal)]) -> Result<RemovalOutcome> { Err(anyhow!(UNSUPPORTED)) }
    fn is_protected(&self, _app: &InstalledApp) -> bool { true }
    fn is_running(&self, _app: &InstalledApp) -> bool { false }
    fn quit(&self, _app: &InstalledApp) -> Result<()> { Err(anyhow!(UNSUPPORTED)) }
    fn cask_status(&self, _app: &InstalledApp, _inv: &[InstalledApp]) -> CaskStatus { CaskStatus::NotManaged }
    fn brew_uninstall(&self, _token: &CaskToken) -> Result<()> { Err(anyhow!(UNSUPPORTED)) }
}
```

Update `core/platform/macos.rs` to the new trait: rename `remove_paths` to `remove_items` taking `&[(PathBuf, Disposal)]` (loop applies the per-item disposal), add `scan(app, _inventory)` (inventory used in Task 5), and add temporary guard methods (`is_running` -> `false`, `quit`/`brew_uninstall` -> `Err`, `cask_status` -> `NotManaged`) to be filled by Task 6.

Update `core/mod.rs` `search`/old `resolve_unique` callers and `cli`/`ui` call sites minimally so the crate compiles (they are reworked in Tasks 8-9; for now have them discover an inventory via `installed_apps()?` and pass it).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p plugin-removeapp`
Expected: PASS. Then `cargo clippy -p plugin-removeapp --all-targets -- -D warnings` clean.

- [ ] **Step 5: Commit**

```bash
git add plugins/plugin-removeapp/src
git commit -m "feat(removeapp): thread inventory, per-item disposal, two-phase removal"
```

---

## Task 5: Owner-aware macOS scan (enumerate + classify + canonical bundle)

**Files:**
- Modify: `plugins/plugin-removeapp/src/core/platform/macos.rs`

**Interfaces:**
- Consumes: `classify::{belongs_to, owner_of, MatchKind}`, the `inventory: &[InstalledApp]`.
- Produces: a `scan` that enumerates Library dirs, classifies each entry, sets `MatchKind`, excludes sibling-owned and protected-owned entries, applies fail-closed rules, and uses canonical paths.

- [ ] **Step 1: Write the failing test**

Add to `core/platform/macos.rs` `tests`:

```rust
    #[test]
    fn scan_includes_helper_excludes_sibling_and_foobar() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let apps = tmp.path().join("Applications");
        write(&apps.join("Foo.app/Contents/Info.plist"), INFO_PLIST_FOO);
        // sibling Bar.app owns com.acme.foo.bar
        let bar = r#"<?xml version="1.0"?><plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>com.acme.foo.bar</string>
<key>CFBundleName</key><string>Bar</string></dict></plist>"#;
        write(&apps.join("Bar.app/Contents/Info.plist"), bar);
        write(&home.join("Library/Caches/com.acme.foo.helper/x"), "x");
        write(&home.join("Library/Caches/com.acme.foo.bar/y"), "y");
        write(&home.join("Library/Caches/com.acme.foobar/z"), "z");

        let plat = Platform::with_roots(home.clone(), vec![apps.clone()]);
        let inventory = plat.installed_apps().unwrap();
        let foo = inventory.iter().find(|a| a.name == "Foo").unwrap().clone();
        let plan = plat.scan(&foo, &inventory).unwrap();
        let paths: Vec<String> = plan.items.iter().map(|l| l.path.to_string_lossy().into_owned()).collect();

        assert!(paths.iter().any(|p| p.ends_with("Caches/com.acme.foo.helper")), "helper kept");
        assert!(!paths.iter().any(|p| p.ends_with("Caches/com.acme.foo.bar")), "sibling excluded");
        assert!(!paths.iter().any(|p| p.ends_with("Caches/com.acme.foobar")), "foobar excluded");
        let helper = plan.items.iter().find(|l| l.path.to_string_lossy().ends_with("foo.helper")).unwrap();
        assert_eq!(helper.match_kind, crate::core::MatchKind::Fuzzy, "non-exact is fuzzy");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p plugin-removeapp scan_includes_helper`
Expected: FAIL - sibling/foobar still present (old exact-construction logic).

- [ ] **Step 3: Write minimal implementation**

In `core/platform/macos.rs`, replace `leftover_candidates` + the body of `scan` with enumerate-then-classify. Define the Library dir set as `(LeftoverKind, subdir, KeyMode)` and classify entries:

```rust
use crate::core::classify::{belongs_to, owner_of, MatchKind};

enum KeyMode { Bundle, Name, Hybrid, SharedExact }

fn library_dirs() -> Vec<(LeftoverKind, &'static str, KeyMode)> {
    use LeftoverKind::*;
    vec![
        (Preferences, "Preferences", KeyMode::Bundle),
        (Containers, "Containers", KeyMode::Bundle),
        (HttpStorages, "HTTPStorages", KeyMode::Bundle),
        (WebKit, "WebKit", KeyMode::Bundle),
        (SavedState, "Saved Application State", KeyMode::Bundle),
        (LaunchAgent, "LaunchAgents", KeyMode::Bundle),
        (ApplicationSupport, "Application Support", KeyMode::Hybrid),
        (Caches, "Caches", KeyMode::Hybrid),
        (Logs, "Logs", KeyMode::Hybrid),
        (GroupContainers, "Group Containers", KeyMode::SharedExact),
    ]
}

fn classify_entry(
    entry: &str,
    app: &InstalledApp,
    all_bids: &[String],
    mode: &KeyMode,
) -> Option<MatchKind> {
    let name_hit = entry.eq_ignore_ascii_case(&app.name);
    let bid_hit = app.bundle_id.as_deref().and_then(|bid| {
        let owner = owner_of(entry, all_bids)?;
        (owner == bid).then(|| if normalize_entry(entry) == bid { MatchKind::Exact } else { MatchKind::Fuzzy })
    });
    match mode {
        KeyMode::Name => name_hit.then_some(MatchKind::Exact),
        KeyMode::Bundle => bid_hit,
        KeyMode::Hybrid => bid_hit.or(name_hit.then_some(MatchKind::Exact)),
        KeyMode::SharedExact => app
            .bundle_id
            .as_deref()
            .filter(|bid| normalize_entry(entry) == *bid)
            .map(|_| MatchKind::Exact),
    }
}
```

`scan` enumerates each dir, calls `classify_entry` per directory entry (using `all_bids` from the inventory), pushes a `Leftover { path: canonical, kind, size_bytes: path_size(&path), match_kind }` for hits, and prepends the app bundle as `MatchKind::Exact`. Import `normalize_entry` from `classify`. Build `all_bids: Vec<String>` from `inventory.iter().filter_map(|a| a.bundle_id.clone())`. Use `std::fs::canonicalize` for stored paths, falling back to the literal path only for the size walk (canonical identity hardening lands in Task 7).

Missing-bundle-id fail-closed: when `app.bundle_id` is `None`, `bid_hit` is `None`, so only `Name`/`Hybrid` exact-name entries match - which is the intended behavior.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p plugin-removeapp` and re-confirm `scan_collects_bundle_and_present_library_leftovers` still passes (adjust it: exact `Caches/com.acme.foo` stays `Exact`).
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add plugins/plugin-removeapp/src/core/platform/macos.rs
git commit -m "feat(removeapp): owner-aware library enumeration with match provenance"
```

---

## Task 6: macOS guards - running (objc2), cask (brew), Spotlight discovery

**Files:**
- Modify: `plugins/plugin-removeapp/src/core/platform/macos.rs`
- Modify: `plugins/plugin-removeapp/Cargo.toml`

**Interfaces:**
- Consumes: `guards::{parse_cask_map, cask_status_for, sanitize_stderr, CaskStatus, CaskToken}`.
- Produces: macOS `is_running`, `quit`, `cask_status`, `brew_uninstall`, and `installed_apps` augmented with `mdfind`.

- [ ] **Step 1: Add dependencies**

In `plugins/plugin-removeapp/Cargo.toml`, under `[target.'cfg(target_os = "macos")'.dependencies]` add:

```toml
objc2 = "0.5"
objc2-app-kit = { version = "0.2", features = ["NSWorkspace", "NSRunningApplication"] }
objc2-foundation = { version = "0.2", features = ["NSString", "NSArray"] }
wait-timeout = "0.2"
```

- [ ] **Step 2: Write the failing test (pure resolver order)**

The objc2/brew calls are thin wrappers (not unit-tested per spec). The testable seam is brew-executable resolution. Add to `macos.rs` a pure `resolve_brew(candidates: &[PathBuf]) -> Option<PathBuf>` and test it:

```rust
    #[test]
    fn resolve_brew_prefers_trusted_paths_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("opt");
        let b = tmp.path().join("usrlocal");
        fs::write(&a, "x").unwrap();
        fs::write(&b, "x").unwrap();
        assert_eq!(resolve_brew(&[a.clone(), b.clone()]), Some(a));
        let missing = tmp.path().join("nope");
        assert_eq!(resolve_brew(&[missing, b.clone()]), Some(b));
        assert_eq!(resolve_brew(&[tmp.path().join("x")]), None);
    }
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p plugin-removeapp resolve_brew`
Expected: FAIL - `resolve_brew` not found.

- [ ] **Step 4: Implement guards and discovery**

Add to `macos.rs`:

```rust
use std::process::Command;
use std::time::Duration;

use crate::core::guards::{cask_status_for, parse_cask_map, sanitize_stderr, CaskStatus, CaskToken};

const BREW_TIMEOUT: Duration = Duration::from_secs(5);
const STDERR_CAP: usize = 4096;

fn resolve_brew(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.exists()).cloned()
}

fn brew_path() -> Option<PathBuf> {
    resolve_brew(&[
        PathBuf::from("/opt/homebrew/bin/brew"),
        PathBuf::from("/usr/local/bin/brew"),
    ])
}

fn run_brew(brew: &Path, args: &[&str]) -> Result<std::process::Output> {
    use wait_timeout::ChildExt;
    let mut child = Command::new(brew)
        .args(args)
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    match child.wait_timeout(BREW_TIMEOUT)? {
        Some(_) => Ok(child.wait_with_output()?),
        None => {
            let _ = child.kill();
            anyhow::bail!("brew timed out")
        }
    }
}
```

Implement the four trait methods on `Platform`:

```rust
fn is_running(&self, app: &InstalledApp) -> bool {
    let Some(bid) = &app.bundle_id else { return false };
    running_bundle_ids().iter().any(|b| b == bid)
}

fn quit(&self, app: &InstalledApp) -> Result<()> {
    let Some(bid) = &app.bundle_id else { anyhow::bail!("no bundle id") };
    if terminate_bundle_id(bid) { Ok(()) } else { anyhow::bail!("removeapp: could not quit {}", app.name) }
}

fn cask_status(&self, app: &InstalledApp, inventory: &[InstalledApp]) -> CaskStatus {
    let Some(brew) = brew_path() else { return CaskStatus::NotManaged };
    let output = match run_brew(&brew, &["info", "--cask", "--json=v2", "--installed"]) {
        Ok(o) if o.status.success() => o,
        Ok(o) => return CaskStatus::Unavailable(sanitize_stderr(&o.stderr, STDERR_CAP)),
        Err(e) => return CaskStatus::Unavailable(e.to_string()),
    };
    let map = match parse_cask_map(&String::from_utf8_lossy(&output.stdout)) {
        Ok(m) => m,
        Err(e) => return CaskStatus::Unavailable(format!("brew json: {e}")),
    };
    let base = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let inv: Vec<String> = inventory.iter().map(|a| base(&a.path)).collect();
    cask_status_for(&base(&app.path), &map, &inv)
}

fn brew_uninstall(&self, token: &CaskToken) -> Result<()> {
    let Some(brew) = brew_path() else { anyhow::bail!("brew not found") };
    let out = run_brew(&brew, &["uninstall", "--cask", "--", token.as_str()])?;
    if out.status.success() { Ok(()) } else { anyhow::bail!("{}", sanitize_stderr(&out.stderr, STDERR_CAP)) }
}
```

Add objc2 helpers (FFI, not unit-tested):

```rust
fn running_bundle_ids() -> Vec<String> {
    use objc2_app_kit::NSWorkspace;
    let mut out = Vec::new();
    unsafe {
        let ws = NSWorkspace::sharedWorkspace();
        for app in ws.runningApplications().iter() {
            if let Some(bid) = app.bundleIdentifier() {
                out.push(bid.to_string());
            }
        }
    }
    out
}

fn terminate_bundle_id(bid: &str) -> bool {
    use objc2_app_kit::NSWorkspace;
    unsafe {
        let ws = NSWorkspace::sharedWorkspace();
        for app in ws.runningApplications().iter() {
            if app.bundleIdentifier().map(|b| b.to_string()).as_deref() == Some(bid) {
                return app.terminate();
            }
        }
    }
    false
}
```

Augment `installed_apps` to fold in `mdfind` results before the dir-walk, deduped by canonical path:

```rust
fn mdfind_app_paths() -> Vec<PathBuf> {
    let out = Command::new("/usr/bin/mdfind")
        .arg("kMDItemContentType == 'com.apple.application-bundle'")
        .output();
    let Ok(out) = out else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(PathBuf::from)
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("app"))
        .collect()
}
```

In `installed_apps`, after collecting the dir-walk apps, add each `mdfind` path not already present (compare `fs::canonicalize` results), reading its bundle info the same way. `mdfind` failure yields an empty vec, so the dir-walk result stands.

- [ ] **Step 5: Run tests and verify the binary**

Run: `cargo test -p plugin-removeapp` then `cargo build -p plugin-removeapp`.
Then a live smoke (macOS): `./target/debug/removeapp scan "Discord"` should show its plan; `is_running`/cask behavior validated manually.
Expected: tests PASS, build clean.

- [ ] **Step 6: Commit**

```bash
git add plugins/plugin-removeapp/Cargo.toml plugins/plugin-removeapp/src/core/platform/macos.rs
git commit -m "feat(removeapp): running, homebrew, and spotlight platform guards"
```

---

## Task 7: Canonical identity and execution-boundary recheck

**Files:**
- Modify: `plugins/plugin-removeapp/src/core/mod.rs` (real `IdentitySnapshot`, recheck in `remove_with`)
- Modify: `plugins/plugin-removeapp/src/core/platform/macos.rs` (capture snapshots in `scan`, canonical paths, protection on canonical path, refuse symlink bundle)

**Interfaces:**
- Produces: `pub struct IdentitySnapshot { file_type, dev, ino, name, ancestor_symlink }` with `capture(path) -> IdentitySnapshot` and `matches(&self, path) -> bool`; `remove_with` rechecks every planned path + `is_running` before mutating.

- [ ] **Step 1: Write the failing test**

Add to `core/mod.rs` tests:

```rust
    #[test]
    fn recheck_aborts_when_a_planned_path_identity_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("Foo.app");
        std::fs::create_dir_all(&bundle).unwrap();
        let a = app("Foo", "com.acme.foo");
        let app2 = InstalledApp { path: bundle.clone(), ..a };
        let snap = IdentitySnapshot::capture(&bundle);
        let plan = RemovalPlan {
            items: vec![Leftover { path: bundle.clone(), kind: LeftoverKind::AppBundle, size_bytes: 0, match_kind: MatchKind::Exact }],
            app: app2, total_bytes: 0, snapshots: vec![snap],
        };
        std::fs::remove_dir_all(&bundle).unwrap();
        std::fs::write(&bundle, "now a file").unwrap();
        let fake = FakePlat::default();
        let err = remove_with(&fake, &plan, Disposal::Trash, &CaskStatus::NotManaged).unwrap_err();
        assert!(err.to_string().contains("changed"), "aborts on identity change");
        assert!(fake.removed.borrow().is_empty(), "no mutation");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p plugin-removeapp recheck_aborts`
Expected: FAIL - `IdentitySnapshot::capture` is a unit struct; no recheck.

- [ ] **Step 3: Implement snapshot + recheck**

Replace the placeholder `IdentitySnapshot` in `core/mod.rs`:

```rust
use std::os::unix::fs::MetadataExt;

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct IdentitySnapshot {
    pub is_symlink: bool,
    pub is_dir: bool,
    pub dev: u64,
    pub ino: u64,
    pub name: String,
}

impl IdentitySnapshot {
    pub fn capture(path: &std::path::Path) -> IdentitySnapshot {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match std::fs::symlink_metadata(path) {
            Ok(m) => IdentitySnapshot {
                is_symlink: m.file_type().is_symlink(),
                is_dir: m.is_dir(),
                dev: m.dev(),
                ino: m.ino(),
                name,
            },
            Err(_) => IdentitySnapshot { name, ..Default::default() },
        }
    }
    pub fn matches(&self, path: &std::path::Path) -> bool {
        let now = IdentitySnapshot::capture(path);
        now.is_symlink == self.is_symlink
            && now.is_dir == self.is_dir
            && now.dev == self.dev
            && now.ino == self.ino
            && now.name == self.name
    }
}
```

In `remove_with`, before the partition/mutation block, add the recheck:

```rust
    if plat.is_running(&plan.app) {
        anyhow::bail!("removeapp: {} started running; resolve the guard again", plan.app.name);
    }
    for (item, snap) in plan.items.iter().zip(plan.snapshots.iter()) {
        if !snap.matches(&item.path) {
            anyhow::bail!("removeapp: {} changed on disk; aborting", item.path.display());
        }
    }
```

Guard the zip when `snapshots` is empty (fake/CLI-less tests): only recheck when `snapshots.len() == items.len()`.

In `macos.rs` `scan`: populate `plan.snapshots` by `IdentitySnapshot::capture` for each item path (same order as `items`), store canonical paths in items, and in `is_protected` canonicalize first (treat canonicalization failure as protected). Refuse a symlinked app bundle: in `scan`, if the bundle path is a symlink, return `Err` (only the symlink itself may be removed by a deliberate path, out of scope here).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p plugin-removeapp`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add plugins/plugin-removeapp/src/core
git commit -m "feat(removeapp): canonical identity snapshots and pre-mutation recheck"
```

---

## Task 8: CLI guard contract and JSON output

**Files:**
- Modify: `plugins/plugin-removeapp/src/cli/mod.rs`

**Interfaces:**
- Consumes: `core::{installed_apps, resolve_unique, plan, guards, remove}`, `guards::CaskStatus`.
- Produces: `Flags { dry_run, yes, force, quit, brew, trash_anyway, query }`; guard-enforcing `run_remove`; JSON output `{ removed, failed, freed_bytes, brew, dry_run, guard_refused }`.

- [ ] **Step 1: Write the failing test**

Add to `cli/mod.rs` tests:

```rust
    #[test]
    fn parse_flags_reads_guard_switches() {
        let f = parse_flags(&["Foo".into(), "--quit".into(), "--brew".into(), "--trash-anyway".into()]);
        assert!(f.quit && f.brew && f.trash_anyway);
        assert_eq!(f.query.as_deref(), Some("Foo"));
    }

    #[test]
    fn guard_refusal_message_names_required_flag() {
        // running guard tripped, no --quit/--trash-anyway -> refusal text names the flag
        let msg = guard_refusal(true, &CaskStatus::NotManaged, &parse_flags(&["Foo".into(), "--yes".into()]));
        let text = msg.expect("should refuse");
        assert!(text.contains("--quit") || text.contains("--trash-anyway"), "names a flag: {text}");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p plugin-removeapp parse_flags_reads_guard`
Expected: FAIL - fields/`guard_refusal` not found.

- [ ] **Step 3: Implement**

Extend `Flags` and `parse_flags` with `quit`, `brew`, `trash_anyway` (`--quit`, `--brew`, `--trash-anyway`). Add a pure `guard_refusal(running: bool, cask: &CaskStatus, flags: &Flags) -> Option<String>` returning the actionable message when a guard is unresolved and the matching flag is absent (`Managed` needs `--brew` or `--trash-anyway`; `running` needs `--quit` or `--trash-anyway`; `Unavailable`/`NotManaged` never block). Rewrite `run_remove`:

```rust
fn run_remove(flags: &Flags) -> Result<ExitCode> {
    let inventory = core::installed_apps()?;
    let app = core::resolve_unique(&inventory, require_query(flags)?)?;
    let plan = core::plan(&app, &inventory)?;
    let g = core::guards(&app, &inventory);

    if flags.dry_run {
        println!("{}", output_json(&plan, &g, None, true, None));
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(reason) = guard_refusal(g.running, &g.cask, flags) {
        eprintln!("removeapp: {reason}");
        return Ok(ExitCode::from(2));
    }
    if g.running && flags.quit && !flags.trash_anyway {
        core::quit_app(&app)?; // thin wrapper over platform().quit(app)
    }
    let requested = if flags.trash_anyway { Disposal::Trash } else { disposal_from_flags(flags.force) };
    let mut brew_token = None;
    if let CaskStatus::Managed(t) = &g.cask {
        if flags.brew && !flags.trash_anyway {
            core::brew_uninstall(t)?; // removes the bundle; remove() then skips it
            brew_token = Some(t.as_str().to_string());
        }
    }
    if !flags.yes && !confirm(&plan, flags.force)? {
        eprintln!("removeapp: aborted");
        return Ok(ExitCode::from(1));
    }
    let outcome = core::remove_after_brew(&plan, requested, &g.cask, brew_token.is_some())?;
    println!("{}", output_json(&plan, &g, Some(&outcome), false, brew_token.as_deref()));
    Ok(if outcome.failed.is_empty() { ExitCode::SUCCESS } else { ExitCode::from(1) })
}
```

Add `core::quit_app`, `core::brew_uninstall`, and `core::remove_after_brew` thin wrappers in `core/mod.rs` (the latter drops the bundle item from the plan when brew handled it, then calls `remove_with`). Implement `output_json(plan, guards, outcome, dry_run, brew) -> String` producing the exact contract object.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p plugin-removeapp` then a live `./target/debug/removeapp remove "SomeApp" --dry-run` to confirm JSON shape.
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add plugins/plugin-removeapp/src/cli/mod.rs plugins/plugin-removeapp/src/core/mod.rs
git commit -m "feat(removeapp): cli guard contract and json output"
```

---

## Task 9: Picker guard banner and state machine

**Files:**
- Modify: `plugins/plugin-removeapp/src/ui/mod.rs`

**Interfaces:**
- Consumes: `core::{guards, remove_after_brew, quit_app, brew_uninstall}`, `guards::{Guards, CaskStatus}`.
- Produces: `Confirming` mode driven by `Guards`; keys `[Q]`/`[B]`/`[T]`/`[enter]` per the transition table; Done shows freed bytes.

- [ ] **Step 1: Implement the guard-aware confirm flow**

(UI is exercised by hand, not unit tests - per the spec's thin-wrapper rule and the gpui-conventions note that live popup behavior is verified through manual runs / the tracer, not headless window tests.)

In `ui/mod.rs`:
- On entering `Confirming` (from `enter_confirm`), compute and store `guards: Guards` (the view already holds `apps`/inventory; pass it to `core::guards`). Store `quit_failed: Option<String>` and `brew_done: bool`.
- Render the banner above the plan: if `guards.running` -> `[Q] quit & continue`; if `CaskStatus::Managed` -> `[B] brew uninstall`; if `CaskStatus::Unavailable(reason)` -> a dim advisory line; always show `[T] trash anyway`, `esc`.
- `on_key` in `Confirming`:
  - `q` when `guards.running`: call `core::quit_app`; on success re-`core::guards` to clear running, else set `quit_failed`.
  - `b` when `matches!(guards.cask, CaskStatus::Managed(_)) && !guards.running`: call `core::brew_uninstall`, set `brew_done`, then `execute` (Trash the rest).
  - `t`: `execute_trash_anyway()` - forces `Disposal::Trash`, proceeds regardless of guards.
  - `enter`: proceed **only if** `!guards.running` and cask is not `Managed`-unresolved; otherwise no-op (the banner is the hint).
  - `d`/`tab` keep the existing trash/delete toggle, but disabled (no-op) while a guard is unresolved.
- `execute` paths call `core::remove_after_brew(&plan, disposal, &guards.cask, brew_done)`; `Done` shows `format_size(outcome.freed_bytes)` and removed/failed counts.

Re-run the execution-boundary recheck is already inside `core::remove_*`, so the UI gets the abort error and can return to `Confirming` (surface the message in the banner).

- [ ] **Step 2: Build and smoke-test**

Run: `cargo build -p plugin-removeapp`, then `./target/debug/removeapp open` (or via `qol dev`) and exercise: a running app shows `[Q]`; a brew app shows `[B]`; `[enter]` is inert while running; `[T]` always works.
Expected: builds; manual flow matches the transition table.

- [ ] **Step 3: Commit**

```bash
git add plugins/plugin-removeapp/src/ui/mod.rs
git commit -m "feat(removeapp): picker guard banner and state machine"
```

---

## Task 10: Final verification and cleanup

**Files:** none (verification only); fix-ups as needed.

- [ ] **Step 1: Full gate**

Run:
```bash
cargo test -p plugin-removeapp
cargo fmt -p plugin-removeapp -- --check
cargo clippy -p plugin-removeapp --all-targets -- -D warnings
cargo build -p plugin-removeapp
```
Expected: all green.

- [ ] **Step 2: Cross-platform compile check (qol-arch-code)**

Run: `cargo check -p plugin-removeapp --target x86_64-unknown-linux-gnu` if the toolchain is present, else confirm linux/windows stubs match the trait by review. Expected: stubs compile.

- [ ] **Step 3: Live acceptance (macOS)**

- `removeapp scan "<a brew app>"` shows owner-aware leftovers incl. `*.helper`.
- `removeapp remove "<running app>" --yes` refuses with a `--quit`/`--trash-anyway` message (exit 2).
- `removeapp remove "<brew app>" --yes` refuses without `--brew`/`--trash-anyway`.
- `removeapp remove "<plain app>" --dry-run` mutates nothing and prints the JSON contract.

- [ ] **Step 4: Delete the superseded stale plan note / finalize**

If this plan replaced the stale file in place, confirm there is no second stale plan lingering. Commit any fix-ups:

```bash
git add -A plugins/plugin-removeapp
git commit -m "test(removeapp): verify smarter, safer removal end to end"
```

---

## Self-Review

**Spec coverage:**
- Owner-aware discovery + classification table + fail-closed -> Tasks 1, 5.
- Per-item `MatchKind` disposal + two-phase + Unavailable bundle downgrade -> Tasks 2, 4, 7.
- Tri-state `CaskStatus`, `CaskToken`, trusted-path-first, argv-only, `NO_AUTO_UPDATE`, timeout, sanitizer -> Tasks 3, 6.
- Inventory threading (`scan`/`cask_status`/`guards`) -> Task 4 (+ consumed in 5, 6, 8, 9).
- Spotlight + canonical identity + execution-boundary recheck -> Tasks 6, 7.
- Guard state machine + transition table (UI + CLI) -> Tasks 8, 9.
- Stubs/qol-arch-code -> Tasks 4, 10.
- JSON contract + freed bytes -> Tasks 2, 8.

**Type consistency:** `MatchKind`, `CaskToken`, `CaskStatus`, `Guards`, `BasenameOwner`, `IdentitySnapshot`, `Leftover { match_kind }`, `RemovalOutcome { freed_bytes }`, `RemovalPlan { snapshots }`, trait `scan(app, inventory)`/`remove_items`/`cask_status(app, inventory)` are defined once (Tasks 1-4, 7) and reused verbatim downstream.

**Open follow-ups (Spec 2 / deferred):** orphan sweep, per-leftover deselect, multi-select, icons, config surface, Linux real impl, force-kill - intentionally out of scope.
