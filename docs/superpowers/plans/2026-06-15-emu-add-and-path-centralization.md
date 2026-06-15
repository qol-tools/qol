# Add emulators from qol dev / qol emu, on a centralized path convention - Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** Let users register/add QEMU emulator images from qol dev (TUI) and qol emu (CLI), built on a single centralized qol-config path convention.

**Architecture:** Part A centralizes the qol-tray config/data path convention into qol-config as the single source of truth (no behavior change, independently shippable). Part B adds a single designated emu dir, an ImageCandidate model for unregistered images, qemu-img validation + toml_edit registration, a firmware resolve chain so Windows boots via UEFI/OVMF without regressing x86 BIOS, and the o/t/a TUI keys plus qol emu add/open CLI verbs.

**Tech Stack:** Rust, clap, toml + toml_edit 0.25, qemu-img JSON, GuestArch/Firmware enums, the existing qol-cli platform trait + dev_console TUI.

---

## Part A - centralize the path convention

### Task A1: Rule doc for the path convention

This step is a pure docs/rule artifact (Standards Evolution: encode the convention **before** applying it in A2/A3). There is no unit test; verification is that the file is created with the exact content and is picked up as a path-scoped rule. The repo already uses path-scoped frontmatter rule files (`paths:` glob + `---` fences) under `<crate>/.claude/rules/` - see `apps/qol-tray/.claude/rules/backend.md`. The new rule lives in the qol-config crate so it auto-loads whenever a session edits the crate that owns the path API, and is committed (the existing `apps/qol-tray/.claude/rules/*.md` files are git-tracked).

The rule's positive claim is scoped to **config and data dirs only** (namespace + the `config`/`data` mapping; no state dir). It must explicitly list the residual `qol-tray` namespace literals the convention does **not** cover, reproduced verbatim from the surveyed code:

- qol-tray's test-only override branch in `apps/qol-tray/src/paths.rs` (the `QOL_TRAY_TEST_PATH_ROOT` thread-local stack + guards), which joins `qol_config::NAMESPACE` itself rather than calling `data_dir()`.
- the three log-dir literals, whose platform conventions differ from `data_dir`:
  - macOS `apps/qol-tray/src/logging/platform/macos.rs:5` - `h.join("Library/Logs/qol-tray")` (macOS `data_dir` is `Application Support`, not `Library/Logs`; routing through `data_dir` would relocate logs).
  - Windows `apps/qol-tray/src/logging/platform/windows.rs:5` - `d.join("qol-tray/logs")`.
  - temp-dir fallback `apps/qol-tray/src/logging/file_logger.rs:44` - `std::env::temp_dir().join("qol-tray/logs")`.

Files:
- Create: `libs/qol-config/.claude/rules/path-convention.md`

Steps:

- [ ] Step 1: Create the rule doc with its exact content. Write the file verbatim:

```markdown
---
paths:
  - "src/**/*.rs"
  - "Cargo.toml"
---

# Path convention: qol-config is the single source for config and data dirs

`qol-config` is the one source of truth for the `qol-tray`-namespaced **config and
data** directories. New components MUST resolve these dirs through `qol-config`,
never by re-deriving `dirs::config_dir()` / `dirs::data_local_dir()` and joining a
`qol-tray` literal inline.

## The API and the mapping

| Bucket | qol-config function | Resolver it wraps | Namespace |
| --- | --- | --- | --- |
| Data | `qol_config::data_dir()` | `dirs::data_local_dir().or_else(dirs::data_dir)` | `qol-tray/` |
| Data subdir | `qol_config::data_subdir(name)` | `data_dir()/name` | `qol-tray/<name>` |
| Config | `qol_config::config_dir()` | `dirs::config_dir()` | `qol-tray/` |

- The namespace literal lives once, as `qol_config::NAMESPACE` (`"qol-tray"`).
- `data_dir()` is canonical; `base_data_dir()` is a `#[doc(hidden)]` alias kept for
  back-compat. Both return `Option<PathBuf>`.
- `config_roots()` and `plugin_config_paths_from_env()` are the existing shared
  surface (qol-gpui and the config plugins depend on their byte-identical install
  search order) and are NOT changed by this convention.

## There is no state dir

The qol-universe does not use an XDG state dir. Run state (e.g. `run.log`,
`report.json`) stays under `target/`, not under a `data`/`config` dir. Do not add a
state-dir resolver.

## Residual `qol-tray` namespace literals NOT covered

This rule is scoped to config/data dirs. The following `qol-tray` literals are
acknowledged residuals and are intentionally NOT routed through `qol-config`:

- **qol-tray test-only override branch** (`apps/qol-tray/src/paths.rs`). The
  `QOL_TRAY_TEST_PATH_ROOT` thread-local override stack and its guards stay in
  qol-tray; only the production join delegates to `qol_config::data_dir()` /
  `qol_config::config_dir()`. The override branch joins `qol_config::NAMESPACE`
  directly rather than calling the resolver, so external integration tests that set
  `QOL_TRAY_TEST_PATH_ROOT` keep working.
- **Log dirs**, whose platform conventions differ from `data_dir` and so cannot
  route through it:
  - macOS `apps/qol-tray/src/logging/platform/macos.rs` - `Library/Logs/qol-tray`
    (macOS `data_dir` is `Application Support`, not `Library/Logs`; redirecting
    would relocate logs).
  - Windows `apps/qol-tray/src/logging/platform/windows.rs` - `qol-tray/logs`.
  - temp-dir fallback `apps/qol-tray/src/logging/file_logger.rs` -
    `temp_dir()/qol-tray/logs`.

  These are log-class literals; converging them is deferred to a future
  `qol_config` log-namespace helper (out of scope).
```

- [ ] Step 2: Verify the file exists with the expected frontmatter and no stray edits.

```bash
test -f /Users/kaho/repos/private/qol-monorepo/libs/qol-config/.claude/rules/path-convention.md && \
head -5 /Users/kaho/repos/private/qol-monorepo/libs/qol-config/.claude/rules/path-convention.md
```

Expected: the file exists; the first five lines are the `paths:` frontmatter (`---`, `paths:`, the two globs, `---`). No build/test runs for a pure doc artifact (the file lives under `.claude/rules/`, outside cargo's source tree, so it does not touch any Rust crate and `cargo` is unaffected).

- [ ] Step 3: Commit.

```bash
git add libs/qol-config/.claude/rules/path-convention.md
git commit -m "docs(qol-config): encode config/data dir path convention rule"
```

Note: the `git add` is scoped to the single new rule file (not `git add -A` / `git add libs/qol-config`), so it does not sweep in any unrelated working-tree change to `libs/qol-config/src/lib.rs`. This keeps the commit atomic to the docs artifact.

### Task A2: qol-config path API

Add `NAMESPACE`, the canonical `data_dir()`, `config_dir()`, and `data_subdir(name)` to `qol-config`, and turn the existing `base_data_dir()` into a thin `#[doc(hidden)]` alias of `data_dir()`. The canonical `data_dir()` resolves `dirs::data_local_dir().or_else(dirs::data_dir)` joined with `NAMESPACE` (an `Option`), and `config_dir()` resolves `dirs::config_dir()` joined with `NAMESPACE`. A private pure helper `resolve_namespaced(base: Option<PathBuf>) -> Option<PathBuf>` does the join so the resolver mapping is table-testable without touching the real `dirs::` crate (which returns environment-dependent paths). `config_roots`/`plugin_config_paths*` are untouched.

**Files:**
- Modify: `libs/qol-config/src/lib.rs` (lines 9-13 for the const + resolver functions; existing test module at lines 153-236 for the new tests)
- Test: `libs/qol-config/src/lib.rs` (the existing `#[cfg(test)] mod tests` block, lines 153-236)

---

- [ ] **Step 1: Write the failing tests.** Add these three tests inside the existing `mod tests` block in `libs/qol-config/src/lib.rs` (e.g. directly after the `use super::*;` line at line 155, before `without_pinned_install_base_is_the_only_data_root`). They exercise the pure resolver mapping and the alias-equality invariant. `NAMESPACE`, `resolve_namespaced`, `data_dir`, and `base_data_dir` must all be in scope via `super::*`. The `cases` tuples are split across lines to stay rustfmt-clean (the crate is fmt-checked in CI).

```rust
    #[test]
    fn namespaced_resolver_joins_base_with_namespace() {
        let cases = [
            (
                Some(PathBuf::from("/data")),
                Some(PathBuf::from("/data/qol-tray")),
            ),
            (
                Some(PathBuf::from("/home/user/.local/share")),
                Some(PathBuf::from("/home/user/.local/share/qol-tray")),
            ),
            (None, None),
        ];
        for (base, expected) in cases {
            assert_eq!(resolve_namespaced(base.clone()), expected, "base: {base:?}");
        }
    }

    #[test]
    fn data_subdir_appends_under_namespaced_data_dir() {
        let Some(data) = data_dir() else {
            return;
        };
        assert_eq!(data_subdir("emu"), Some(data.join("emu")));
    }

    #[test]
    fn base_data_dir_is_an_alias_of_data_dir() {
        assert_eq!(base_data_dir(), data_dir());
        assert_eq!(NAMESPACE, "qol-tray");
    }
```

- [ ] **Step 2: Run the tests to verify they fail.**

```bash
cargo test -p qol-config --lib tests
```

Expected: FAIL to compile - `cannot find value NAMESPACE in this scope`, `cannot find function resolve_namespaced in this scope`, and `cannot find function data_dir in this scope` / `cannot find function data_subdir in this scope` (none exist yet).

- [ ] **Step 3: Write the minimal implementation.** Replace the current `base_data_dir` definition (lines 9-13) with the const, the canonical `data_dir`, the private `resolve_namespaced` helper, `config_dir`, `data_subdir`, and the `#[doc(hidden)]` alias. Keep `dirs::data_dir` fully qualified so the local `data_dir` fn does not shadow it confusingly. The block to write in place of the old `base_data_dir`:

```rust
pub const NAMESPACE: &str = "qol-tray";

fn resolve_namespaced(base: Option<PathBuf>) -> Option<PathBuf> {
    base.map(|path| path.join(NAMESPACE))
}

pub fn data_dir() -> Option<PathBuf> {
    resolve_namespaced(dirs::data_local_dir().or_else(dirs::data_dir))
}

pub fn config_dir() -> Option<PathBuf> {
    resolve_namespaced(dirs::config_dir())
}

pub fn data_subdir(name: &str) -> Option<PathBuf> {
    data_dir().map(|path| path.join(name))
}

#[doc(hidden)]
pub fn base_data_dir() -> Option<PathBuf> {
    data_dir()
}
```

- [ ] **Step 4: Run the tests to verify they pass, then confirm fmt/build/lints.**

```bash
cargo test -p qol-config --lib tests
```

Expected: PASS - the three new tests plus the six pre-existing `assemble_config_roots` tests are green (the `tests` filter matches the whole `mod tests`, so the full lib test set runs and all pass). Then confirm formatting, the whole crate, and lints are clean:

```bash
cargo fmt -p qol-config --check
cargo build -p qol-config
cargo clippy -p qol-config --all-targets -- -D warnings
```

Expected: PASS - fmt-clean, clean build, no clippy warnings. (`config_roots` at line 16 still calls `base_data_dir()`, which now delegates to `data_dir()`, so the install search order is byte-identical.)

- [ ] **Step 5: Commit.**

```bash
git add libs/qol-config/src/lib.rs
git commit -m "feat(config): add data_dir/config_dir/data_subdir path API with base_data_dir alias"
```

### Task A3: Migrate qol-cli/qol-tray to qol-config; delete dead consts

> Prerequisite: A2 must have landed in `libs/qol-config/src/lib.rs` the symbols this step consumes: `pub const NAMESPACE: &str = "qol-tray"`, `pub fn data_dir() -> Option<PathBuf>`, `pub fn config_dir() -> Option<PathBuf>`, `pub fn data_subdir(name: &str) -> Option<PathBuf>`. As of this writing the real `libs/qol-config/src/lib.rs` exposes only `pub fn base_data_dir() -> Option<PathBuf>` (note: NOT `data_dir`) and has no `NAMESPACE`/`config_dir`/`data_subdir`. A3 references the A2 symbols as pre-existing, so A3 is blocked until A2 ships them. The qol-tray crate already depends on qol-config (`qol-config.workspace = true` at `apps/qol-tray/Cargo.toml:97`); only `tools/qol-cli/Cargo.toml` needs the dependency added.
>
> TDD note: in tasks A3b, A3d, A3e, and A3f the migration is behavior-preserving - the old hardcoded literal is exactly `"qol-tray"`, which equals `qol_config::NAMESPACE`. So those new tests PASS against the pre-migration code; they are regression guards, not red-first tests. The only genuinely-failing-first test is A3c, where the helper `active_worktree_marker_path` does not exist yet. Steps below are labeled accordingly.
>
> Style: the codebase is comment-free. No inline comments appear in any impl code below.

#### Task A3a: Add qol-config dependency to qol-cli

**Files:**
- Modify: `tools/qol-cli/Cargo.toml:13-20` ([dependencies] block)

This is a pure dependency add (no unit test); verify by build. The workspace root defines `qol-config = { path = "libs/qol-config" }`, so `qol-config.workspace = true` is valid.

- [ ] Step 1: Add the dependency. Edit `tools/qol-cli/Cargo.toml`, changing the `[dependencies]` block from:
```toml
[dependencies]
anyhow = "1.0"
dirs = "6.0"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
toml.workspace = true
ratatui = "0.30"
ansi-to-tui = "8.0"
```
to:
```toml
[dependencies]
anyhow = "1.0"
dirs = "6.0"
qol-config.workspace = true
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
toml.workspace = true
ratatui = "0.30"
ansi-to-tui = "8.0"
```

- [ ] Step 2: Verify the crate still builds with the new edge.
```bash
cargo build -p qol
```
Expected: PASS (qol-config compiles into the dependency graph; no code uses it yet. qol-cli enables no `unused_crate_dependencies` lint, so an unused path dep is not a `-D warnings` failure).

- [ ] Step 3: Commit.
```bash
git add tools/qol-cli/Cargo.toml
git commit -m "build(qol-cli): depend on qol-config"
```

#### Task A3b: Migrate qol-cli emu_config_path to qol_config

**Files:**
- Modify: `tools/qol-cli/src/commands/emu.rs:223-225` (`emu_config_path`)
- Test: `tools/qol-cli/src/commands/emu.rs` (tests module at line 1047, `use super::*;`)

`emu_config_path` is `pub(crate)`; `PathBuf` is already imported at `emu.rs:10` (`use std::path::{Path, PathBuf};`). `dirs` stays live (`dirs::home_dir()` at emu.rs:689/692).

- [ ] Step 1: Add the regression-guard test inside the existing `#[cfg(test)] mod tests` block (after the last test, before the closing brace):
```rust
#[test]
fn emu_config_path_is_under_qol_config_namespace() {
    let path = emu_config_path().expect("config dir resolves in test env");
    assert!(
        path.ends_with("emu.toml"),
        "expected emu.toml leaf, got {path:?}"
    );
    let parent = path.parent().expect("emu.toml has a parent");
    assert!(
        parent.ends_with(qol_config::NAMESPACE),
        "expected parent under {} namespace, got {parent:?}",
        qol_config::NAMESPACE
    );
}
```

- [ ] Step 2: Run the test. Note: because the migration is behavior-preserving (the old literal `"qol-tray/emu.toml"` already nests under the same namespace dir), this test PASSES against the current `emu_config_path` once A3a's edge is in place. It is a regression guard that locks the namespace contract, not a red-first test.
```bash
cargo test -p qol commands::emu::tests::emu_config_path_is_under_qol_config_namespace
```
Expected: PASS even before the impl change.

- [ ] Step 3: Replace `emu_config_path` in `tools/qol-cli/src/commands/emu.rs`:
```rust
pub(crate) fn emu_config_path() -> Option<PathBuf> {
    qol_config::config_dir().map(|dir| dir.join("emu.toml"))
}
```

- [ ] Step 4: Re-run the test; it stays green and now proves the path is routed through `qol_config`.
```bash
cargo test -p qol commands::emu::tests::emu_config_path_is_under_qol_config_namespace
```
Expected: PASS.

- [ ] Step 5: Verify no warnings (`dirs` is still used at emu.rs:689/692, so it stays a live dependency).
```bash
cargo clippy -p qol --all-targets -- -D warnings
```
Expected: PASS.

- [ ] Step 6: Commit.
```bash
git add tools/qol-cli/src/commands/emu.rs
git commit -m "refactor(qol-cli): resolve emu config path via qol-config"
```

#### Task A3c: Migrate qol-cli dev active-worktree marker to qol_config

**Files:**
- Modify: `tools/qol-cli/src/commands/dev.rs:11` (imports), `:226-232` (`clear_active_worktree_marker`)
- Test: `tools/qol-cli/src/commands/dev.rs` (tests module at line 333)

`clear_active_worktree_marker` returns `()` and ignores errors (best-effort `remove_file`); it is not directly unit-tested today. Introduce a private path helper that is testable and have the marker clearer call it. This is the one A3 task with a true failing-first test (the helper does not exist yet).

- [ ] Step 1: Write the failing test. Add to the `#[cfg(test)] mod tests` block (which already contains `parses_worktree_branches_skipping_detached`):
```rust
#[test]
fn active_worktree_marker_path_is_under_qol_config_namespace() {
    let path = active_worktree_marker_path().expect("config dir resolves in test env");
    assert!(
        path.ends_with("dev/active-worktree.txt"),
        "expected dev/active-worktree.txt tail, got {path:?}"
    );
    let namespaced = path
        .components()
        .any(|c| c.as_os_str() == qol_config::NAMESPACE);
    assert!(namespaced, "expected {} in {path:?}", qol_config::NAMESPACE);
}
```

- [ ] Step 2: Run the test to verify it fails.
```bash
cargo test -p qol commands::dev::tests::active_worktree_marker_path_is_under_qol_config_namespace
```
Expected: FAIL to compile - `active_worktree_marker_path` does not exist yet.

- [ ] Step 3: Write minimal implementation. In `tools/qol-cli/src/commands/dev.rs`, change the import at line 11 from:
```rust
use std::path::Path;
```
to:
```rust
use std::path::{Path, PathBuf};
```
Then replace `clear_active_worktree_marker` (lines 226-232):
```rust
fn active_worktree_marker_path() -> Option<PathBuf> {
    qol_config::config_dir().map(|dir| dir.join("dev/active-worktree.txt"))
}

fn clear_active_worktree_marker() {
    let Some(path) = active_worktree_marker_path() else {
        return;
    };
    let _ = std::fs::remove_file(path);
}
```

- [ ] Step 4: Run the test to verify it passes.
```bash
cargo test -p qol commands::dev::tests::active_worktree_marker_path_is_under_qol_config_namespace
```
Expected: PASS.

- [ ] Step 5: Verify no warnings.
```bash
cargo clippy -p qol --all-targets -- -D warnings
```
Expected: PASS.

- [ ] Step 6: Commit.
```bash
git add tools/qol-cli/src/commands/dev.rs
git commit -m "refactor(qol-cli): resolve dev worktree marker via qol-config"
```

#### Task A3d: Migrate qol-tray paths.rs production joins to qol_config; delete APP_NAME

**Files:**
- Modify: `apps/qol-tray/src/paths.rs:7` (delete `APP_NAME`), `:90-99` (`legacy_config_dir`), `:105-115` (`base_data_dir`)
- Test: `apps/qol-tray/src/paths.rs` (tests module at line 335; `paths_have_correct_suffixes` at line 339 already asserts `qol-tray` containment)

The production branch is centralized; the test-root override branch stays but references `qol_config::NAMESPACE` instead of the deleted `APP_NAME`. The existing guard tests (`paths_have_correct_suffixes`, etc.) drive the override branch and must stay green. After replacing both fns, `APP_NAME` has no remaining references (currently used only at lines 93/98/108/114), so deleting the const is clean.

- [ ] Step 1: Add the regression-guard test to the `#[cfg(test)] mod tests` block. `push_test_path_root`, `base_data_dir`, `shared_config_dir`, and `TempDir` are all in scope:
```rust
#[test]
fn override_branch_nests_under_qol_config_namespace() {
    let tmp = TempDir::new().unwrap();
    let _guard = push_test_path_root(tmp.path());

    let data = base_data_dir().unwrap();
    assert!(
        data.ends_with(format!("data/{}", qol_config::NAMESPACE)),
        "data dir {data:?} should nest data/<namespace>"
    );

    let config = shared_config_dir().unwrap();
    assert!(
        config.ends_with(format!("config/{}", qol_config::NAMESPACE)),
        "config dir {config:?} should nest config/<namespace>"
    );
}
```

- [ ] Step 2: Run the test. The override branch currently joins `APP_NAME` (= `"qol-tray"`), which equals `qol_config::NAMESPACE`, and qol-tray already depends on qol-config, so this PASSES against the current code. It is a regression guard, not a red-first test.
```bash
cargo test -p qol-tray paths::tests::override_branch_nests_under_qol_config_namespace
```
Expected: PASS even before the impl change.

- [ ] Step 3: Write minimal implementation. In `apps/qol-tray/src/paths.rs`:

Delete the dead const at line 7:
```rust
const APP_NAME: &str = "qol-tray";
```

Replace `legacy_config_dir` (lines 90-99):
```rust
fn legacy_config_dir() -> Result<PathBuf> {
    #[cfg(any(test, debug_assertions))]
    if let Some(root) = test_path_root() {
        return Ok(root.join("config").join(qol_config::NAMESPACE));
    }

    qol_config::config_dir().context("Could not determine config directory")
}
```

Replace `base_data_dir` (lines 105-115):
```rust
pub(crate) fn base_data_dir() -> Result<PathBuf> {
    #[cfg(any(test, debug_assertions))]
    if let Some(root) = test_path_root() {
        return Ok(root.join("data").join(qol_config::NAMESPACE));
    }

    qol_config::data_dir().context("Could not determine local data directory")
}
```

- [ ] Step 4: Run the new guard plus the unchanged override guards to confirm green.
```bash
cargo test -p qol-tray paths::tests::override_branch_nests_under_qol_config_namespace paths::tests::paths_have_correct_suffixes
```
Expected: PASS for both.

- [ ] Step 5: Verify no warnings. `Context` from anyhow is still used via `.context(...)`. The replaced production branches were the only `dirs::` calls in this file and were fully path-qualified (no `use dirs` to remove); `dirs` stays a live crate dep via other qol-tray modules.
```bash
cargo clippy -p qol-tray --all-targets -- -D warnings
```
Expected: PASS.

- [ ] Step 6: Commit.
```bash
git add apps/qol-tray/src/paths.rs
git commit -m "refactor(qol-tray): delegate base/config dir to qol-config and drop APP_NAME"
```

#### Task A3e: Migrate doctor/install_id active path to qol_config; delete APP_NAME

**Files:**
- Modify: `apps/qol-tray/src/doctor/install_id.rs:7` (delete `APP_NAME`), `:21-26` (`active_install_id_path`)
- Test: `apps/qol-tray/src/doctor/install_id.rs` (add `#[cfg(test)] mod tests`)

`active_install_id_path` is `pub(super)` and consumed by `install_identity.rs`. It joins `APP_NAME` + `ACTIVE_INSTALL_ID_FILE` onto the data dir. After migration it routes the data dir through `qol_config::data_dir()`; `ACTIVE_INSTALL_ID_FILE` and `INSTALL_ID_MARKER_FILE` stay (the latter still used by `marker_path_for`). `APP_NAME` is used only at line 25 (inside the replaced fn), so deletion is clean. The imports `anyhow::{anyhow, Context, Result}` all stay live (`anyhow!` at line 13, `bail!` at line 39, `Context`/`.with_context` at line 44, `Context`/`.context` in the new impl).

- [ ] Step 1: Add a tests module at the end of `apps/qol-tray/src/doctor/install_id.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_install_id_path_nests_namespace_and_marker_file() {
        let path = active_install_id_path().expect("data dir resolves in test env");
        assert!(
            path.ends_with(ACTIVE_INSTALL_ID_FILE),
            "expected {ACTIVE_INSTALL_ID_FILE} leaf, got {path:?}"
        );
        let parent = path.parent().expect("active-install-id has a parent");
        assert!(
            parent.ends_with(qol_config::NAMESPACE),
            "expected parent under {} namespace, got {parent:?}",
            qol_config::NAMESPACE
        );
    }
}
```

- [ ] Step 2: Run the test. The current `active_install_id_path` joins `APP_NAME` (= `"qol-tray"` = `qol_config::NAMESPACE`) onto the data dir, and qol-tray already depends on qol-config, so this PASSES against the current code. Regression guard, not red-first.
```bash
cargo test -p qol-tray doctor::install_id::tests::active_install_id_path_nests_namespace_and_marker_file
```
Expected: PASS even before the impl change.

- [ ] Step 3: Write minimal implementation. In `apps/qol-tray/src/doctor/install_id.rs`:

Delete the dead const at line 7:
```rust
const APP_NAME: &str = "qol-tray";
```

Replace `active_install_id_path` (lines 21-26):
```rust
pub(super) fn active_install_id_path() -> Result<PathBuf> {
    let base = qol_config::data_dir().context("could not determine local data directory")?;
    Ok(base.join(ACTIVE_INSTALL_ID_FILE))
}
```

- [ ] Step 4: Run the new guard plus the install_identity checks that consume this path; all stay green.
```bash
cargo test -p qol-tray doctor::install_id::tests::active_install_id_path_nests_namespace_and_marker_file
cargo test -p qol-tray doctor::checks::install_identity
```
Expected: PASS.

- [ ] Step 5: Verify no warnings (`Context` still used via `.context(...)`; `anyhow`/`bail`/`Result` imports unchanged and still used; the removed `dirs::` calls were path-qualified with no `use dirs` to drop).
```bash
cargo clippy -p qol-tray --all-targets -- -D warnings
```
Expected: PASS.

- [ ] Step 6: Commit.
```bash
git add apps/qol-tray/src/doctor/install_id.rs
git commit -m "refactor(qol-tray): resolve doctor install-id path via qol-config and drop APP_NAME"
```

#### Task A3f: Route task_runner fallback through qol_config (clear the last namespace literal)

**Files:**
- Modify: `apps/qol-tray/src/features/task_runner/config.rs:50-55` (`fallback_config_path`)
- Test: `apps/qol-tray/src/features/task_runner/config.rs` (extend the existing tests module at line 102)

The DoD grep would otherwise still flag `config.rs:53` (`.join("qol-tray")`). Migrating the fallback to `qol_config::config_dir()` removes that literal with no behavioral change (`config_path()` still prefers the profile path and only falls back here). `CONFIG_FILENAME` stays; `PathBuf` is already imported at line 3.

- [ ] Step 1: Add the test to the existing `#[cfg(test)] mod tests` block (line 102, `use super::*;`):
```rust
#[test]
fn fallback_config_path_uses_qol_config_namespace_when_available() {
    let path = fallback_config_path();
    assert!(
        path.ends_with(CONFIG_FILENAME),
        "expected {CONFIG_FILENAME} leaf, got {path:?}"
    );
    if path != PathBuf::from(".").join(CONFIG_FILENAME) {
        let parent = path.parent().expect("config path has a parent");
        assert!(
            parent.ends_with(qol_config::NAMESPACE),
            "expected parent under {} namespace, got {parent:?}",
            qol_config::NAMESPACE
        );
    }
}
```

- [ ] Step 2: Run the test. The current `fallback_config_path` joins `"qol-tray"` (= `qol_config::NAMESPACE`) before `CONFIG_FILENAME`, and qol-tray already depends on qol-config, so this PASSES against the current code. Regression guard, not red-first.
```bash
cargo test -p qol-tray features::task_runner::config::tests::fallback_config_path_uses_qol_config_namespace_when_available
```
Expected: PASS even before the impl change.

- [ ] Step 3: Replace `fallback_config_path` in `apps/qol-tray/src/features/task_runner/config.rs`:
```rust
fn fallback_config_path() -> PathBuf {
    qol_config::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(CONFIG_FILENAME)
}
```

- [ ] Step 4: Re-run the test; it stays green and now proves the fallback routes through `qol_config`.
```bash
cargo test -p qol-tray features::task_runner::config::tests::fallback_config_path_uses_qol_config_namespace_when_available
```
Expected: PASS.

- [ ] Step 5: Verify no warnings (the removed `dirs::config_dir()` was path-qualified; `PathBuf` import unchanged; `qol-config` already a qol-tray dependency).
```bash
cargo clippy -p qol-tray --all-targets -- -D warnings
```
Expected: PASS.

- [ ] Step 6: Commit.
```bash
git add apps/qol-tray/src/features/task_runner/config.rs
git commit -m "refactor(qol-tray): route task-runner fallback config dir via qol-config"
```

#### Task A3g: Verify the DoD namespace grep and full green

**Files:** none (verification only).

- [ ] Step 1: Run the DoD grep. The unanchored regex `join("qol-tray` also matches installer artifact names (`qol-tray.desktop`/`.png`/`.icns`/`.cmd`, `Programs/qol-tray/bin`), monorepo repo-dir joins (`self_build.rs`, `plugin_store/.../worktrees.rs`, `plugin_store/installer/operations.rs`, `workspace.rs`, `dev_mock_handlers.rs`), and a test fixture (`tests/plugin_action_dispatch_e2e.rs`) - none of which are config/data namespace literals and none of which A3 migrates. Scope the grep to the config/data resolution surface by excluding those trees:
```bash
rg 'join\("qol-tray|join\(APP_NAME' \
  --glob '!**/target/**' \
  --glob '!**/docs/**' \
  --glob '!**/tests/**' \
  --glob '!**/installer/**' \
  --glob '!**/dev/**' \
  --glob '!**/plugin_store/**' \
  --glob '!**/workspace.rs' \
  libs/qol-config apps/qol-tray tools/qol-cli
```
Expected after A3a–A3f land - exactly these residuals, all acceptable:
- `libs/qol-config/src/lib.rs` (two hits: `path.join("qol-tray")` and `dirs::config_dir().map(|p| p.join("qol-tray"))`) - the source-of-truth namespace owner.
- `apps/qol-tray/src/logging/file_logger.rs` - `join("qol-tray/logs")` (logging is not migrated; uniform residual).
- `apps/qol-tray/src/logging/platform/windows.rs` - `join("qol-tray/logs")` (same).

No `join(APP_NAME)` hits remain (both consts deleted). The migrated sites (`emu.rs`, `dev.rs`, `paths.rs` override+production, `doctor/install_id.rs`, `task_runner/config.rs`) now use `qol_config::NAMESPACE` / `qol_config::config_dir()` / `qol_config::data_dir()` and do not match. If any line other than the four residuals above appears, it was missed and must be migrated before proceeding.

(Running the bare grep from the design-spec DoD without the extra excludes will also surface installer/repo-dir/test matches; those are pre-existing non-namespace literals out of A3 scope, not regressions.)

- [ ] Step 2: Confirm both crates build, test, and lint clean together.
```bash
cargo build -p qol -p qol-tray
cargo test -p qol -p qol-tray
cargo clippy -p qol -p qol-tray --all-targets -- -D warnings
```
Expected: PASS for build, test, and clippy. The qol-tray test-root override guard tests (`paths_have_correct_suffixes`, `active_profile_name_*`, `profile_dirs_reflect_active_profile`, `switching_profile_changes_resolved_paths`, etc.) stay green, confirming the wrap-not-move override is intact.

- [ ] Step 3: No commit (verification-only task; all changes were committed in A3a–A3f).

## Part B - add-emulator flow

### Task B1: Emu dir resolution + single-dir scan + legacy advisory

> Depends on A2/A3 having landed: `qol_config::data_subdir(name: &str) -> Option<PathBuf>` exists and `qol-config = { workspace = true }` is in `tools/qol-cli/Cargo.toml`. The crate package name for qol-cli is `qol`.
>
> VERIFY BEFORE STARTING: as of the current tree neither precondition is met - `libs/qol-config/src/lib.rs` exposes `base_data_dir()` but no `data_subdir`, and `tools/qol-cli/Cargo.toml` has no `qol-config` dependency. The workspace root `Cargo.toml` does declare `qol-config = { path = "libs/qol-config" }`, so the `workspace = true` form is correct once added. Do not start B1 until A2/A3 have actually landed both pieces; otherwise Part 5 (`qol_config::data_subdir("emu")`) will not compile.

**Files:**
- Modify: `tools/qol-cli/src/commands/emu/discovery/filesystem.rs` (9-53 scan/discover; add walk helper + legacy count; tests 84-103)
- Modify: `tools/qol-cli/src/commands/emu/discovery/config.rs` (1-7 imports already provide `TomlValue`/`PathBuf`; widen `expand_home` to `pub(crate)`; add `parse_emu_dir`; tests 87-152)
- Modify: `tools/qol-cli/src/commands/emu/discovery/mod.rs` (11-33 `DiscoveryContext` + `discover` + re-exports)
- Modify: `tools/qol-cli/src/commands/emu.rs` (223-301 `emu_config_path`/`cmd_list`/`cmd_doctor`; 686-694 `discover_environments`; add `emu_dir` + `legacy_advisory`)
- Modify: `tools/qol-cli/src/dev_console.rs` (1947-1961 `draw_emu` empty branch)
- Test: same files (`#[cfg(test)] mod tests`)

---

#### Part 1 - Extract the path-collecting walk helper in `filesystem.rs`

- [ ] **Step 1: Write the failing test** - append to the `tests` module in `tools/qol-cli/src/commands/emu/discovery/filesystem.rs` (after line 102, before the closing `}`):

```rust
    #[test]
    fn collect_image_paths_walks_recursively_and_dedupes_non_images() {
        let root = std::env::temp_dir().join(format!("qol-emu-walk-{}", std::process::id()));
        let nested = root.join("sub");
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join("a.qcow2"), b"x").unwrap();
        fs::write(root.join("notes.txt"), b"x").unwrap();
        fs::write(nested.join("b.img"), b"x").unwrap();

        let mut seen = HashSet::new();
        let mut paths = collect_image_paths(&[root.clone()], &mut seen);
        paths.sort();

        assert_eq!(paths.len(), 2, "paths: {paths:?}");
        assert!(paths.iter().any(|p| p.ends_with("a.qcow2")), "paths: {paths:?}");
        assert!(paths.iter().any(|p| p.ends_with("b.img")), "paths: {paths:?}");
        fs::remove_dir_all(&root).unwrap();
    }
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p qol commands::emu::discovery::filesystem::tests::collect_image_paths_walks_recursively_and_dedupes_non_images
```

Expected: FAIL - `cannot find function collect_image_paths in this scope` (compile error).

- [ ] **Step 3: Write minimal implementation** - replace the body of `filesystem.rs` lines 9-53 (the `discover` + `collect_image_environments` block) with the extracted walk helper plus a thin `collect_image_environments` that consumes it. New file region (keeping `is_vm_image_path` and `image_id` below unchanged):

```rust
pub(crate) fn discover(dir: &Path) -> Vec<Environment> {
    let mut seen = HashSet::new();
    let paths = collect_image_paths(std::slice::from_ref(&dir.to_path_buf()), &mut seen);
    collect_image_environments(paths)
}

fn collect_image_environments(paths: Vec<PathBuf>) -> Vec<Environment> {
    paths
        .into_iter()
        .map(|path| {
            let id = image_id(&path);
            Environment {
                name: humanize_id(&id),
                id,
                backend: "qemu".to_string(),
                arch: GuestArch::X86_64,
                image_path: path,
                source: "scan".to_string(),
            }
        })
        .collect()
}

pub(crate) fn collect_image_paths(roots: &[PathBuf], seen: &mut HashSet<PathBuf>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for root in roots {
        collect_into(root, MAX_SCAN_DEPTH, seen, &mut paths);
    }
    paths
}

fn collect_into(root: &Path, depth: usize, seen: &mut HashSet<PathBuf>, paths: &mut Vec<PathBuf>) {
    if depth == 0 || !root.is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_into(&path, depth - 1, seen, paths);
            continue;
        }
        if !is_vm_image_path(&path) {
            continue;
        }
        let canonical = path.canonicalize().unwrap_or(path);
        if seen.insert(canonical.clone()) {
            paths.push(canonical);
        }
    }
}
```

Note the signature change: `discover` now takes `dir: &Path` (was `&[PathBuf]`). The `super::super::{...}` import line at the top already provides `GuestArch`, `humanize_id`, `sanitize_id`, `Environment`; `humanize_id` stays used by `collect_image_environments` and `sanitize_id` stays used by the unchanged `image_id` below, so no unused-import warning.

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p qol commands::emu::discovery::filesystem::tests::collect_image_paths_walks_recursively_and_dedupes_non_images
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tools/qol-cli/src/commands/emu/discovery/filesystem.rs
git commit -m "refactor(emu): extract path-collecting image walk helper"
```

---

#### Part 2 - `legacy_root_image_count` routing through `platform::image_search_roots`, excluding registered

- [ ] **Step 1: Write the failing test** - append to the `tests` module in `tools/qol-cli/src/commands/emu/discovery/filesystem.rs`:

```rust
    #[test]
    fn legacy_count_excludes_registered_canonical_paths() {
        let root = std::env::temp_dir().join(format!("qol-emu-legacy-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.qcow2"), b"x").unwrap();
        fs::write(root.join("b.img"), b"x").unwrap();

        let mut seen = HashSet::new();
        let walked = collect_image_paths(&[root.clone()], &mut seen);
        assert_eq!(walked.len(), 2, "walked: {walked:?}");

        let mut all = HashSet::new();
        assert_eq!(count_unregistered(&[root.clone()], &all), 2);

        all.insert(walked[0].clone());
        assert_eq!(count_unregistered(&[root.clone()], &all), 1);
        fs::remove_dir_all(&root).unwrap();
    }
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p qol commands::emu::discovery::filesystem::tests::legacy_count_excludes_registered_canonical_paths
```

Expected: FAIL - `cannot find function count_unregistered in this scope`.

- [ ] **Step 3: Write minimal implementation** - add to `tools/qol-cli/src/commands/emu/discovery/filesystem.rs` after `collect_image_paths` (the public count helper plus the testable pure inner function):

```rust
pub(crate) fn legacy_root_image_count(registered: &HashSet<PathBuf>) -> usize {
    let roots = super::super::platform::image_search_roots(dirs::home_dir());
    count_unregistered(&roots, registered)
}

fn count_unregistered(roots: &[PathBuf], registered: &HashSet<PathBuf>) -> usize {
    let mut seen = HashSet::new();
    collect_image_paths(roots, &mut seen)
        .into_iter()
        .filter(|path| !registered.contains(path))
        .count()
}
```

`platform::image_search_roots(home: Option<PathBuf>)` is verified present on all three OS impls, so the keep-alive chain holds. `dirs` is already a direct dependency of the `qol` crate (`dirs = "6.0"`), so `dirs::home_dir()` resolves here without a new import. `collect_image_paths` already canonicalizes each path, and `registered` is built from canonical Environment paths (Part 4), so the exclusion shares discovery's canonical basis.

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p qol commands::emu::discovery::filesystem::tests::legacy_count_excludes_registered_canonical_paths
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tools/qol-cli/src/commands/emu/discovery/filesystem.rs
git commit -m "feat(emu): count-only legacy-root scan excluding registered images"
```

---

#### Part 3 - Parse the top-level `dir` key in `config.rs`

- [ ] **Step 1: Write the failing test** - append to the `tests` module in `tools/qol-cli/src/commands/emu/discovery/config.rs`:

```rust
    #[test]
    fn parses_top_level_dir_with_home_expansion() {
        let home = PathBuf::from("/home/me");
        let cases = [
            ("dir = \"~/vms\"\n", Some(PathBuf::from("/home/me/vms"))),
            ("dir = \"/srv/vms\"\n", Some(PathBuf::from("/srv/vms"))),
            ("[images]\n", None),
        ];
        for (content, expected) in cases {
            assert_eq!(parse_emu_dir(content, Some(&home)), expected, "content: {content}");
        }
    }
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p qol commands::emu::discovery::config::tests::parses_top_level_dir_with_home_expansion
```

Expected: FAIL - `cannot find function parse_emu_dir in this scope`.

- [ ] **Step 3: Write minimal implementation** - first widen the existing private `expand_home` to `pub(crate)` (B1 crosses the module boundary by re-exporting it for later tasks; do this in B1, not B3 which only widens `parse_image_overrides`). Change its signature line in `config.rs` from `fn expand_home(` to:

```rust
pub(crate) fn expand_home(value: &str, home: Option<&PathBuf>) -> PathBuf {
```

Then add to `tools/qol-cli/src/commands/emu/discovery/config.rs` after `parse_image_overrides` (line 69), reusing `expand_home`. `TomlValue` and `PathBuf` are already imported at the top of the file (lines 4-5), so no new imports are needed:

```rust
pub(crate) fn parse_emu_dir(content: &str, home: Option<&PathBuf>) -> Option<PathBuf> {
    let parsed: TomlValue = toml::from_str(content).ok()?;
    let dir = parsed.get("dir").and_then(TomlValue::as_str)?;
    Some(expand_home(dir, home))
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p qol commands::emu::discovery::config::tests::parses_top_level_dir_with_home_expansion
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tools/qol-cli/src/commands/emu/discovery/config.rs
git commit -m "feat(emu): parse top-level dir key from emu.toml"
```

---

#### Part 4 - `DiscoveryContext` carries `emu_dir`, replacing `image_search_roots`

- [ ] **Step 1: Write the failing test** - append a new `tests` module at the end of `tools/qol-cli/src/commands/emu/discovery/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;

    #[test]
    fn discover_scans_the_single_emu_dir() {
        let dir = std::env::temp_dir().join(format!("qol-emu-ctx-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("win11.qcow2"), b"x").unwrap();

        let environments = discover(DiscoveryContext {
            config_path: None,
            home_dir: None,
            virsh: None,
            libvirt_uris: &[],
            emu_dir: dir.clone(),
        })
        .unwrap();

        assert_eq!(environments.len(), 1, "environments: {environments:?}");
        assert_eq!(environments[0].source, "scan");

        let mut registered = HashSet::new();
        registered.insert(environments[0].image_path.clone());
        assert!(registered.iter().any(|p| p.ends_with("win11.qcow2")));
        fs::remove_dir_all(&dir).unwrap();
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p qol commands::emu::discovery::tests::discover_scans_the_single_emu_dir
```

Expected: FAIL - `struct DiscoveryContext has no field named emu_dir` (compile error).

- [ ] **Step 3: Write minimal implementation** - edit `tools/qol-cli/src/commands/emu/discovery/mod.rs`. Replace the `image_search_roots` field with `emu_dir`, change the `filesystem::discover` call to pass `&context.emu_dir`, and re-export the count helper alongside `is_vm_image_path`:

```rust
use anyhow::Result;
use std::path::PathBuf;

use super::Environment;

mod config;
mod dedupe;
mod filesystem;
mod libvirt;

pub(crate) use filesystem::{is_vm_image_path, legacy_root_image_count};

pub(crate) struct DiscoveryContext {
    pub(crate) config_path: Option<PathBuf>,
    pub(crate) home_dir: Option<PathBuf>,
    pub(crate) virsh: Option<PathBuf>,
    pub(crate) libvirt_uris: &'static [&'static str],
    pub(crate) emu_dir: PathBuf,
}

pub(crate) fn discover(context: DiscoveryContext) -> Result<Vec<Environment>> {
    let mut environments = Vec::new();
    environments.extend(config::discover(
        context.config_path.as_deref(),
        context.home_dir.as_ref(),
    )?);
    environments.extend(libvirt::discover(
        context.virsh.as_deref(),
        context.libvirt_uris,
    ));
    environments.extend(filesystem::discover(&context.emu_dir));
    Ok(dedupe::dedupe_and_sort(environments))
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo test -p qol commands::emu::discovery::tests::discover_scans_the_single_emu_dir
```

Expected: PASS. (The crate as a whole will not yet build - `emu.rs:692` still constructs the old `image_search_roots` field; fixed in Part 5. This part's green gate is at the end of Part 6.)

- [ ] **Step 5: Commit** - defer; this part does not compile in isolation (the `emu.rs` caller updates land in Part 5). Continue to Part 5 before committing.

---

#### Part 5 - `emu_dir()` resolver + wire `discover_environments`

- [ ] **Step 1: Write the failing test** - add a resolver case. Since `emu_dir()` reads the real config path / data dir, test the pure override-vs-fallback decision via a small helper `resolve_emu_dir`. Append to the existing top-level `#[cfg(test)] mod tests` in `tools/qol-cli/src/commands/emu.rs` (the module opens at line 1048; test fns are indented 4 spaces - match that exactly so `cargo fmt --check` stays clean):

```rust
    #[test]
    fn resolve_emu_dir_prefers_parsed_override() {
        let parsed = Some(PathBuf::from("/home/me/vms"));
        let fallback = Some(PathBuf::from("/data/qol-tray/emu"));
        let cases = [
            (parsed.clone(), fallback.clone(), Some(PathBuf::from("/home/me/vms"))),
            (None, fallback.clone(), Some(PathBuf::from("/data/qol-tray/emu"))),
            (None, None, None),
        ];
        for (override_dir, default_dir, expected) in cases {
            assert_eq!(
                resolve_emu_dir(override_dir.clone(), default_dir.clone()),
                expected,
                "override: {override_dir:?}, default: {default_dir:?}"
            );
        }
    }
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p qol commands::emu::tests::resolve_emu_dir_prefers_parsed_override
```

Expected: FAIL - `cannot find function resolve_emu_dir in this scope`.

- [ ] **Step 3: Write minimal implementation** - in `tools/qol-cli/src/commands/emu.rs` add the resolver functions after `emu_config_path` (ends at line 225), rewrite `discover_environments` (lines 686-694) to use `emu_dir()`, and import the parse helper.

First, expand the `use discovery::DiscoveryContext;` line (line 26) to also import the count and parse helpers:

```rust
use discovery::{legacy_root_image_count, parse_emu_dir, DiscoveryContext};
```

Then re-export `parse_emu_dir` from `discovery/mod.rs` by adding a `config` re-export line next to the `filesystem` one:

```rust
pub(crate) use config::parse_emu_dir;
pub(crate) use filesystem::{is_vm_image_path, legacy_root_image_count};
```

Now add to `emu.rs` after `emu_config_path` (after line 225). `Path` and `PathBuf` are already imported (line 10) and `fs` (line 8), `dirs` is a dependency:

```rust
fn emu_dir() -> Option<PathBuf> {
    let override_dir = emu_config_path()
        .filter(|path| path.is_file())
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|content| parse_emu_dir(&content, dirs::home_dir().as_ref()));
    resolve_emu_dir(override_dir, qol_config::data_subdir("emu"))
}

fn resolve_emu_dir(override_dir: Option<PathBuf>, default_dir: Option<PathBuf>) -> Option<PathBuf> {
    override_dir.or(default_dir)
}
```

Then rewrite `discover_environments` (lines 686-694), dropping `image_search_roots`:

```rust
fn discover_environments() -> Result<Vec<Environment>> {
    discovery::discover(DiscoveryContext {
        config_path: emu_config_path(),
        home_dir: dirs::home_dir(),
        virsh: find_on_path("virsh"),
        libvirt_uris: platform::libvirt_uris(),
        emu_dir: emu_dir().unwrap_or_default(),
    })
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo build -p qol && cargo test -p qol commands::emu::tests::resolve_emu_dir_prefers_parsed_override
```

Expected: PASS (and the crate now builds - `dev_console.rs` still compiles because `discover_environments`'s signature is unchanged).

- [ ] **Step 5: Commit**

```bash
git add tools/qol-cli/src/commands/emu/discovery/mod.rs tools/qol-cli/src/commands/emu/discovery/filesystem.rs tools/qol-cli/src/commands/emu/discovery/config.rs tools/qol-cli/src/commands/emu.rs
git commit -m "feat(emu): scan a single resolved emu dir, retain legacy count helper"
```

---

#### Part 6 - Persistent legacy advisory wired into list-empty, doctor, and TUI empty state

- [ ] **Step 1: Write the failing test** - append to the top-level `#[cfg(test)] mod tests` in `tools/qol-cli/src/commands/emu.rs` (4-space indentation):

```rust
    #[test]
    fn legacy_advisory_renders_only_for_nonzero_count() {
        let dir = PathBuf::from("/data/qol-tray/emu");
        assert_eq!(legacy_advisory(0, &dir), None);
        let line = legacy_advisory(3, &dir).expect("nonzero count yields advisory");
        assert!(line.contains("3 image"), "line: {line}");
        assert!(line.contains("qol emu add"), "line: {line}");
        assert!(line.contains("/data/qol-tray/emu"), "line: {line}");
    }
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p qol commands::emu::tests::legacy_advisory_renders_only_for_nonzero_count
```

Expected: FAIL - `cannot find function legacy_advisory in this scope`.

- [ ] **Step 3: Write minimal implementation** - add the advisory text helper to `tools/qol-cli/src/commands/emu.rs` (after `emu_dir`/`resolve_emu_dir`), then wire it into the three render sites.

Add the helper plus a registered-set collector:

```rust
fn legacy_advisory(count: usize, emu_dir: &Path) -> Option<String> {
    if count == 0 {
        return None;
    }
    Some(format!(
        "{count} image(s) found in legacy roots (~/VMs, ...); run `qol emu add <path>` to register, or move them into {}",
        emu_dir.display()
    ))
}

fn registered_image_paths() -> Result<std::collections::HashSet<PathBuf>> {
    Ok(discover_environments()?
        .into_iter()
        .map(|environment| environment.image_path)
        .collect())
}
```

Wire into `cmd_list`'s empty branch (replace lines 234-239):

```rust
    if statuses.is_empty() {
        step_label("env", StepKind::Info, "no emus found");
        if let Some(path) = emu_config_path() {
            step_label("config", StepKind::Info, &path.display().to_string());
        }
        if let Some(dir) = emu_dir() {
            let count = legacy_root_image_count(&registered_image_paths()?);
            if let Some(advisory) = legacy_advisory(count, &dir) {
                step_label("legacy", StepKind::Info, &advisory);
            }
        }
        return Ok(());
    }
```

Wire into `cmd_doctor` (insert after the `found`/`runs` block, before `Ok(())` at line 300):

```rust
    if let Some(dir) = emu_dir() {
        step_label("emu-dir", StepKind::Info, &dir.display().to_string());
        let count = legacy_root_image_count(&registered_image_paths()?);
        if let Some(advisory) = legacy_advisory(count, &dir) {
            step_label("legacy", StepKind::Info, &advisory);
        }
    }
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo build -p qol && cargo test -p qol commands::emu::tests::legacy_advisory_renders_only_for_nonzero_count
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tools/qol-cli/src/commands/emu.rs
git commit -m "feat(emu): persistent legacy-root advisory in list and doctor"
```

---

#### Part 7 - Wire the advisory into the dev-console emu empty state

- [ ] **Step 1: Write the failing test** - the `draw_emu` empty branch renders via ratatui, so test the pure line-builder. Add a pure helper `emu_empty_lines` and a test. Append to the top-level `#[cfg(test)] mod tests` in `tools/qol-cli/src/dev_console.rs` (module opens at line 2635; test fns are 4-space indented):

```rust
    #[test]
    fn emu_empty_lines_include_advisory_when_legacy_present() {
        let with = emu_empty_lines(
            "~/.config/qol-tray/emu.toml",
            Some("2 image(s) in legacy roots".to_string()),
        );
        assert_eq!(with.len(), 3, "lines: {with:?}");
        let without = emu_empty_lines("~/.config/qol-tray/emu.toml", None);
        assert_eq!(without.len(), 2, "lines: {without:?}");
    }
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p qol dev_console::tests::emu_empty_lines_include_advisory_when_legacy_present
```

Expected: FAIL - `cannot find function emu_empty_lines in this scope`.

- [ ] **Step 3: Write minimal implementation** - in `tools/qol-cli/src/dev_console.rs`, extract the empty-state line builder and call it from `draw_emu`. First add the new emu helpers to the existing `use crate::commands::emu::{...}` block (lines 16-19):

```rust
use crate::commands::emu::{
    emu_config_path, emu_dir, environment_statuses, legacy_advisory_count, newest_run_detail,
    EnvironmentStatus, LastRun, ResolveState, RunDetail,
};
```

This needs two `pub(crate)` shims in `emu.rs`. Add to `tools/qol-cli/src/commands/emu.rs` (next to `legacy_advisory`):

```rust
pub(crate) fn legacy_advisory_count() -> Option<String> {
    let dir = emu_dir()?;
    let registered = registered_image_paths().ok()?;
    legacy_advisory(legacy_root_image_count(&registered), &dir)
}
```

and make `emu_dir` reachable by changing its signature (defined in Part 5) to `pub(crate)`:

```rust
pub(crate) fn emu_dir() -> Option<PathBuf> {
```

Then in `dev_console.rs` add the pure builder above `draw_emu` (before line 1947). `Line`, `Color`, and `Stylize` (`.fg`) are already imported (lines 11-12), and `.fg()` is verified to work on `String`:

```rust
fn emu_empty_lines(config: &str, advisory: Option<String>) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from("  no emus found".fg(Color::DarkGray)),
        Line::from(vec![
            "  config ".fg(Color::DarkGray),
            config.to_string().fg(Color::White),
        ]),
    ];
    if let Some(advisory) = advisory {
        lines.push(Line::from(vec![
            "  legacy ".fg(Color::DarkGray),
            advisory.fg(Color::Yellow),
        ]));
    }
    lines
}
```

and replace the `draw_emu` empty branch (lines 1950-1961) with - note this is one match arm of the existing `match &dash.emu`, so the surrounding `Probing`/`Done`/`Failed` arms stay intact:

```rust
        EmuState::Done(statuses) if statuses.is_empty() => {
            let config = emu_config_path()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "~/.config/qol-tray/emu.toml".to_string());
            emu_empty_lines(&config, legacy_advisory_count())
        }
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo build -p qol && cargo test -p qol dev_console::tests::emu_empty_lines_include_advisory_when_legacy_present
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tools/qol-cli/src/commands/emu.rs tools/qol-cli/src/dev_console.rs
git commit -m "feat(emu): show legacy-root advisory in dev console empty state"
```

---

#### Part 8 - Whole-crate green gate

- [ ] **Step 1: Run the full emu/discovery test set**

```bash
cargo test -p qol commands::emu
```

Expected: PASS (all `filesystem`, `config`, `discovery`, and `emu` tests, including the six pre-existing emu tests at emu.rs:1051-1234).

- [ ] **Step 2: Build, clippy, fmt with `-D warnings`** (CI gates on this; the `legacy_root_image_count` -> `platform::image_search_roots` keep-alive chain must keep all three per-OS impls reachable so none trips `dead_code`, and the dropped `DiscoveryContext.image_search_roots` field must leave no unused-field warning):

```bash
cargo build -p qol && cargo clippy -p qol --all-targets -- -D warnings && cargo fmt -p qol -- --check
```

Expected: clean - no `dead_code` on `image_search_roots`, no unused-field or unused-import warnings.

- [ ] **Step 3: Commit** - nothing to commit if the previous parts left the tree green; otherwise run `cargo fmt -p qol` then:

```bash
git add -A tools/qol-cli/src
git commit -m "style(emu): fmt after emu dir resolution wiring"
```

### Task B2: ImageCandidate / Firmware types + Discovered owning constructor

This step adds the `Firmware` enum (alongside `GuestArch` in `arch.rs`, re-exported the same way `GuestArch` is), the `ImageCandidate` struct, and the `Discovered` struct with its single owning constructor `Discovered::partition`. The constructor receives the already-deduped merged registered set (output of `dedupe_and_sort`) plus the raw `emu_dir` filesystem entries, and computes only the candidate/environment split: it drops any `emu_dir` entry whose canonical path matches a registered `Environment`'s canonical `image_path`. It reuses `is_vm_image_path` + `Path::canonicalize` (the same rule as `dedupe_and_sort`), does NOT stand up a second canonical dedup over the merged set, and keeps `is_vm_image_path` exported and signature-stable for its `machine.rs:50` consumer.

B2 does NOT do arch/firmware filename inference (that is B3): candidates default `arch = GuestArch::X86_64`, `arch_inferred = false`, and `firmware = Firmware::for_arch(arch)`. B3 replaces these defaults with real inference.

Note on test invocation: the `qol` package is a **binary-only crate** (`tools/qol-cli/Cargo.toml` declares `[[bin]] name = "qol"` and no `[lib]`). Unit tests live in the binary target, so `cargo test -p qol --lib` fails with `error: no library targets found in package qol`. Use `cargo test -p qol --bin qol <filter>` (equivalently the CI form runs the bin tests under `--all-targets`).

Files:
- Modify: `tools/qol-cli/src/commands/emu/arch.rs` (add `Firmware` enum + impl after the `impl GuestArch` block, which closes at line 45; before `#[cfg(test)]` at line 47 / `mod tests {` at line 48)
- Modify: `tools/qol-cli/src/commands/emu.rs` (line 25 re-export block: add `Firmware` to the `pub(crate) use arch::...` re-export)
- Create: `tools/qol-cli/src/commands/emu/discovery/candidate.rs` (new module: `ImageCandidate`, `Discovered`, `Discovered::partition`, tests)
- Modify: `tools/qol-cli/src/commands/emu/discovery/mod.rs` (lines 1-11: register `mod candidate;`, re-export `Discovered`, `ImageCandidate`; keep `is_vm_image_path` re-export untouched)
- Confirm only: `tools/qol-cli/src/commands/emu/discovery/filesystem.rs` (`image_id` at line 65 is already `pub(crate)` and `is_vm_image_path` at line 55 is already `pub(crate)`; no change needed, verified during build)
- Test: `tools/qol-cli/src/commands/emu/arch.rs` `#[cfg(test)] mod tests`
- Test: `tools/qol-cli/src/commands/emu/discovery/candidate.rs` `#[cfg(test)] mod tests`

---

- [ ] Step 1: Write the failing test for `Firmware` (as_str / parse round-trip + arch default). Append these two tests inside the existing `#[cfg(test)] mod tests` block in `tools/qol-cli/src/commands/emu/arch.rs` (after `qemu_system_binary_is_arch_suffixed`, before the closing `}` of the test module).

```rust
    #[test]
    fn firmware_as_str_and_parse_round_trip() {
        let cases = [
            (Firmware::Bios, "bios"),
            (Firmware::Uefi, "uefi"),
        ];
        for (firmware, expected) in cases {
            assert_eq!(firmware.as_str(), expected, "firmware: {firmware:?}");
            assert_eq!(Firmware::parse(expected), Some(firmware), "input: {expected}");
        }
        assert_eq!(Firmware::parse("legacy"), None);
        assert_eq!(Firmware::parse(""), None);
    }

    #[test]
    fn firmware_for_arch_defaults_uefi_on_arm_bios_on_x86() {
        let cases = [
            (GuestArch::X86_64, Firmware::Bios),
            (GuestArch::Aarch64, Firmware::Uefi),
        ];
        for (arch, expected) in cases {
            assert_eq!(Firmware::for_arch(arch), expected, "arch: {arch:?}");
        }
    }
```

- [ ] Step 2: Run the test to verify it fails (Firmware does not yet exist).

```bash
cargo test -p qol --bin qol commands::emu::arch::tests::firmware 2>&1 | tail -20
```

Expected: FAIL - compile error `cannot find type/value Firmware in this scope` (the `Firmware` enum and its `as_str`/`parse`/`for_arch` are not defined yet).

- [ ] Step 3: Write the minimal implementation. Add the `Firmware` enum and impl in `tools/qol-cli/src/commands/emu/arch.rs`, immediately after the closing `}` of the `impl GuestArch` block (which ends at line 45, just before the `#[cfg(test)]` attribute at line 47).

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Firmware {
    Bios,
    Uefi,
}

impl Firmware {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Firmware::Bios => "bios",
            Firmware::Uefi => "uefi",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Firmware> {
        match value {
            "bios" => Some(Firmware::Bios),
            "uefi" => Some(Firmware::Uefi),
            _ => None,
        }
    }

    pub(crate) fn for_arch(arch: GuestArch) -> Firmware {
        match arch {
            GuestArch::X86_64 => Firmware::Bios,
            GuestArch::Aarch64 => Firmware::Uefi,
        }
    }
}
```

- [ ] Step 4: Run the test to verify it passes.

```bash
cargo test -p qol --bin qol commands::emu::arch::tests::firmware 2>&1 | tail -20
```

Expected: PASS - `firmware_as_str_and_parse_round_trip` and `firmware_for_arch_defaults_uefi_on_arm_bios_on_x86` both green.

- [ ] Step 5: Commit.

```bash
git add tools/qol-cli/src/commands/emu/arch.rs
git commit -m "feat(emu): add Firmware enum with as_str/parse and arch default"
```

---

- [ ] Step 6: Re-export `Firmware` from the emu module so `ImageCandidate` (and B3's `Environment`) can name it. In `tools/qol-cli/src/commands/emu.rs`, change the arch re-export at line 25.

```rust
pub(crate) use arch::{Firmware, GuestArch};
```

- [ ] Step 7: Build to verify the re-export resolves (no test yet; this is a visibility change consumed by Step 8).

```bash
cargo build -p qol 2>&1 | tail -20
```

Expected: builds clean. In the non-test build `Firmware` is referenced only by the test added in Step 1, so under `-D warnings` the `pub(crate) use arch::Firmware` re-export would be flagged as `unused_imports`; the next steps add the real consumer (`ImageCandidate`). A plain `cargo build` (no `-D warnings`) succeeds with at most a warning. If an `unused_imports` warning appears here, proceed to Step 10 which consumes it (do not commit this step alone).

---

- [ ] Step 8: Create the failing-test module AND register it. Create `tools/qol-cli/src/commands/emu/discovery/candidate.rs` with ONLY the test module first (implementation lands in Step 10), and register `mod candidate;` in `tools/qol-cli/src/commands/emu/discovery/mod.rs` so the file is actually compiled. (Do NOT add the `pub(crate) use candidate::{...}` re-export yet - the types do not exist, so a re-export would fail to resolve; the test below uses `use super::*` instead. The re-export is added in Step 11.) Use absolute, generic paths and a tempdir so canonicalization works.

First, in `tools/qol-cli/src/commands/emu/discovery/mod.rs`, add `mod candidate;` to the existing module list (leave the `is_vm_image_path` re-export and `discover()` untouched):

```rust
mod candidate;
mod config;
mod dedupe;
mod filesystem;
mod libvirt;

pub(crate) use filesystem::is_vm_image_path;
```

Then create `tools/qol-cli/src/commands/emu/discovery/candidate.rs` with the test module only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_env(id: &str, path: &std::path::Path) -> Environment {
        Environment {
            id: id.to_string(),
            name: humanize_id(id),
            backend: "qemu".to_string(),
            arch: GuestArch::X86_64,
            image_path: path.to_path_buf(),
            source: "config".to_string(),
        }
    }

    #[test]
    fn partition_excludes_registered_paths_and_keeps_unregistered_candidates() {
        let root = std::env::temp_dir().join(format!("qol-emu-b2-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let registered_file = root.join("registered.qcow2");
        let candidate_file = root.join("fresh.qcow2");
        let not_an_image = root.join("notes.txt");
        for file in [&registered_file, &candidate_file, &not_an_image] {
            fs::write(file, b"x").unwrap();
        }

        let environments = vec![make_env("registered", &registered_file)];
        let entries = vec![
            registered_file.clone(),
            candidate_file.clone(),
            not_an_image.clone(),
        ];

        let discovered = Discovered::partition(environments, &entries);

        assert_eq!(discovered.environments.len(), 1, "registered env preserved");
        assert_eq!(discovered.environments[0].id, "registered");
        assert_eq!(
            discovered.candidates.len(),
            1,
            "only the unregistered image is a candidate"
        );
        let candidate = &discovered.candidates[0];
        assert_eq!(candidate.path, candidate_file.canonicalize().unwrap());
        assert_eq!(candidate.id, "fresh");
        assert_eq!(candidate.display_name, "Fresh");
        assert_eq!(candidate.arch, GuestArch::X86_64);
        assert!(!candidate.arch_inferred);
        assert_eq!(candidate.firmware, Firmware::Bios);

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn partition_dedups_repeated_entries_and_registered_into_emu_dir() {
        let root = std::env::temp_dir().join(format!("qol-emu-b2dup-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let registered_image = root.join("vm.qcow2");
        let plain_image = root.join("plain.qcow2");
        for file in [&registered_image, &plain_image] {
            fs::write(file, b"x").unwrap();
        }

        let environments = vec![make_env("vm", &registered_image)];
        let entries = vec![
            registered_image.clone(),
            plain_image.clone(),
            plain_image.clone(),
        ];

        let discovered = Discovered::partition(environments, &entries);

        assert_eq!(
            discovered.candidates.len(),
            1,
            "registered-into-emu_dir not double-listed, repeated entry collapsed"
        );
        assert_eq!(discovered.candidates[0].id, "plain");

        fs::remove_dir_all(&root).unwrap();
    }
}
```

- [ ] Step 9: Run the test to verify it fails to compile (the module is now compiled, but the types/constructor do not exist).

```bash
cargo test -p qol --bin qol commands::emu::discovery::candidate 2>&1 | tail -20
```

Expected: FAIL - compile errors `cannot find type Discovered`, `cannot find type ImageCandidate`, `cannot find value Firmware` in module scope (the `use super::*` brings in nothing because no definitions exist yet, and the implementation `use` imports are not present until Step 10).

- [ ] Step 10: Write the minimal implementation. Prepend the imports, `ImageCandidate`, `Discovered`, and `Discovered::partition` to `tools/qol-cli/src/commands/emu/discovery/candidate.rs` (above the `#[cfg(test)] mod tests` block added in Step 8).

```rust
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::super::{arch::Firmware, arch::GuestArch, humanize_id, Environment};
use super::filesystem::{image_id, is_vm_image_path};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImageCandidate {
    pub(crate) id: String,
    pub(crate) path: PathBuf,
    pub(crate) display_name: String,
    pub(crate) arch: GuestArch,
    pub(crate) arch_inferred: bool,
    pub(crate) firmware: Firmware,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Discovered {
    pub(crate) environments: Vec<Environment>,
    pub(crate) candidates: Vec<ImageCandidate>,
}

impl Discovered {
    pub(crate) fn partition(environments: Vec<Environment>, emu_dir_entries: &[PathBuf]) -> Self {
        let registered: HashSet<PathBuf> = environments
            .iter()
            .map(|environment| canonical(&environment.image_path))
            .collect();
        let mut seen = HashSet::new();
        let mut candidates = Vec::new();
        for entry in emu_dir_entries {
            if !is_vm_image_path(entry) {
                continue;
            }
            let path = canonical(entry);
            if registered.contains(&path) {
                continue;
            }
            if !seen.insert(path.clone()) {
                continue;
            }
            candidates.push(candidate_for(path));
        }
        Self {
            environments,
            candidates,
        }
    }
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn candidate_for(path: PathBuf) -> ImageCandidate {
    let id = image_id(&path);
    let display_name = humanize_id(&id);
    let arch = GuestArch::X86_64;
    ImageCandidate {
        id,
        path,
        display_name,
        arch,
        arch_inferred: false,
        firmware: Firmware::for_arch(arch),
    }
}
```

- [ ] Step 11: Re-export the new types in `tools/qol-cli/src/commands/emu/discovery/mod.rs`. The `mod candidate;` line was added in Step 8; now add the `pub(crate) use candidate::{Discovered, ImageCandidate};` re-export next to the existing `is_vm_image_path` re-export (leave that line and `discover()` untouched).

```rust
mod candidate;
mod config;
mod dedupe;
mod filesystem;
mod libvirt;

pub(crate) use candidate::{Discovered, ImageCandidate};
pub(crate) use filesystem::is_vm_image_path;
```

- [ ] Step 12: Run the test to verify it passes.

```bash
cargo test -p qol --bin qol commands::emu::discovery::candidate 2>&1 | tail -20
```

Expected: PASS - `partition_excludes_registered_paths_and_keeps_unregistered_candidates` and `partition_dedups_repeated_entries_and_registered_into_emu_dir` both green.

- [ ] Step 13: Verify the type-definition commit compiles and clippy is clean under `-D warnings`. `Discovered`, `Discovered::partition`, and `ImageCandidate` are constructed only in `#[cfg(test)]` tests at this point; their production consumers (`discover`/`discover_all`/`emu_scan` in B2e, `infer_candidate`/`register_image` in B3) land later, so add `#[allow(dead_code)]` to them in this commit (B2e and B3 make them live). Run all three and read the output.

```bash
cargo build -p qol 2>&1 | tail -20
cargo test -p qol --bin qol commands::emu 2>&1 | tail -20
cargo clippy -p qol --all-targets -- -D warnings 2>&1 | tail -30
```

Expected: PASS - build green, emu tests green, clippy green. Note: CI runs clippy with `--all-targets` (verified: `.github/scripts/affected_crates.py` always appends `--all-targets`, and `.github/workflows/ci.yml:89` runs `cargo clippy $CLIPPY_ARGS -- -D warnings`). Under `--all-targets` the test harness compiles, so `Discovered`/`ImageCandidate`/`Firmware` count as used through the tests and clippy stays green. A bare `cargo clippy -p qol -- -D warnings` (no `--all-targets`) may flag `Discovered`/`ImageCandidate` as never-constructed in the non-test build; that is expected at B2 (their production caller arrives in B5). The gating command for this step is the `--all-targets` form above, which is the CI form.

- [ ] Step 14: Commit.

```bash
git add tools/qol-cli/src/commands/emu.rs tools/qol-cli/src/commands/emu/discovery/mod.rs tools/qol-cli/src/commands/emu/discovery/candidate.rs
git commit -m "feat(emu): add ImageCandidate/Discovered with owning partition constructor"
```


The defect this fixes: after B1, `discovery::discover` still ran `environments.extend(filesystem::discover(&context.emu_dir))`, so every `emu_dir` image became a scan-`Environment` and `Discovered::partition` was never called. The single-splitter fix below removes the filesystem `Environment` producers, changes `discover()` to partition (config + libvirt deduped) against the raw `emu_dir` entries, reshapes the `emu.rs` callers around one discovery pass, and `pub(crate)`-re-exports `Discovered`/`ImageCandidate` from `emu.rs` so the dev console can name them.

#### Task B2e: Rewire discovery to produce candidates

This task removes the filesystem `Environment`-producing pair, repoints `discovery::discover` at `Discovered::partition`, reshapes `emu.rs` so environments and candidates come from one discovery pass, and re-exports the candidate types crate-wide. It also moves the B1 `discover_scans_the_single_emu_dir` test onto the `Discovered` contract.

This is a pure rewire across a signature change (`discover` returns `Discovered`, not `Vec<Environment>`) with one new pure-function seam (`statuses_for`). The signature-change parts have no failing-unit-test seam, so their verification is `cargo build -p qol` plus `cargo clippy -p qol --all-targets -- -D warnings` plus the relevant `cargo test -p qol --bin qol` run. Implementation code is comment-free.

Files:
- Modify: `tools/qol-cli/src/commands/emu/discovery/filesystem.rs` (remove `discover` + `collect_image_environments`; drop the now-unused `humanize_id`/`sanitize_id`/`GuestArch`/`Environment` imports; keep `collect_image_paths`, `is_vm_image_path`, `image_id`)
- Modify: `tools/qol-cli/src/commands/emu/discovery/mod.rs` (`discover` signature + body; the `discover_scans_the_single_emu_dir` test from B1)
- Modify: `tools/qol-cli/src/commands/emu.rs` (add `discover_all`; refactor `discover_environments`; extract `statuses_for` from `environment_statuses`; add `emu_scan`; `pub(crate)`-re-export `Discovered`/`ImageCandidate`)

---

#### Step 1: Remove the filesystem `Environment` producers

This is a deletion plus import trim across a signature removal; verify with build + clippy. `collect_image_paths`, `is_vm_image_path`, and `image_id` stay; only the `Environment`-producing pair is removed. The `humanize_id`/`sanitize_id`/`GuestArch`/`Environment` imports that B1 left in the `super::super::{...}` line were used solely by `collect_image_environments`, so they must be dropped or `-D warnings` flags `unused_imports`.

**Before** (`filesystem.rs`, after B1):

```rust
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::super::{arch::GuestArch, humanize_id, sanitize_id, Environment};

const MAX_SCAN_DEPTH: usize = 4;

pub(crate) fn discover(dir: &Path) -> Vec<Environment> {
    let mut seen = HashSet::new();
    let paths = collect_image_paths(std::slice::from_ref(&dir.to_path_buf()), &mut seen);
    collect_image_environments(paths)
}

fn collect_image_environments(paths: Vec<PathBuf>) -> Vec<Environment> {
    paths
        .into_iter()
        .map(|path| {
            let id = image_id(&path);
            Environment {
                name: humanize_id(&id),
                id,
                backend: "qemu".to_string(),
                arch: GuestArch::X86_64,
                image_path: path,
                source: "scan".to_string(),
            }
        })
        .collect()
}

pub(crate) fn collect_image_paths(roots: &[PathBuf], seen: &mut HashSet<PathBuf>) -> Vec<PathBuf> {
```

**After** (`filesystem.rs`): the `discover` + `collect_image_environments` block is gone, and the `use super::super::{...}` line is removed entirely:

```rust
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_SCAN_DEPTH: usize = 4;

pub(crate) fn collect_image_paths(roots: &[PathBuf], seen: &mut HashSet<PathBuf>) -> Vec<PathBuf> {
```

`collect_image_paths`, `collect_into`, `is_vm_image_path`, and `image_id` reference none of `Environment`/`GuestArch`/`humanize_id`/`sanitize_id`. (`legacy_root_image_count` from B1 references `super::super::platform::image_search_roots` by full path, not via this `use`.) `Path`/`PathBuf`/`HashSet`/`fs` all stay used by the surviving functions.

**Verify**: this step does not compile in isolation (`mod.rs` still calls `filesystem::discover`); the green gate is at the end of Step 5. Defer the commit; continue to Step 2.

---

#### Step 2: `discovery::discover` returns `Discovered` via `partition`

Build `environments` from config + libvirt only, run the existing `dedupe::dedupe_and_sort` over that merged set (no filesystem extend), collect the raw `emu_dir` entries with `collect_image_paths`, and hand both to `Discovered::partition`. `dedupe_and_sort` already canonicalizes and de-dups the merged set, and `partition` excludes any `emu_dir` entry whose canonical path matches a registered `Environment`'s, so a config/libvirt disk that resolves into `emu_dir` lists once as the `Environment`, never as a candidate.

**Before** (`mod.rs`, after B1+B2 typedefs):

```rust
use anyhow::Result;
use std::path::PathBuf;

use super::Environment;

mod candidate;
mod config;
mod dedupe;
mod filesystem;
mod libvirt;

pub(crate) use candidate::{Discovered, ImageCandidate};
pub(crate) use config::parse_emu_dir;
pub(crate) use filesystem::{is_vm_image_path, legacy_root_image_count};

pub(crate) struct DiscoveryContext {
    pub(crate) config_path: Option<PathBuf>,
    pub(crate) home_dir: Option<PathBuf>,
    pub(crate) virsh: Option<PathBuf>,
    pub(crate) libvirt_uris: &'static [&'static str],
    pub(crate) emu_dir: PathBuf,
}

pub(crate) fn discover(context: DiscoveryContext) -> Result<Vec<Environment>> {
    let mut environments = Vec::new();
    environments.extend(config::discover(
        context.config_path.as_deref(),
        context.home_dir.as_ref(),
    )?);
    environments.extend(libvirt::discover(
        context.virsh.as_deref(),
        context.libvirt_uris,
    ));
    environments.extend(filesystem::discover(&context.emu_dir));
    Ok(dedupe::dedupe_and_sort(environments))
}
```

**After** (`mod.rs`): add `HashSet` to the imports, change `discover`'s return type to `Result<Discovered>`, drop the filesystem extend, and partition. The `pub(crate) use candidate::{Discovered, ImageCandidate};` re-export is unchanged.

```rust
use anyhow::Result;
use std::collections::HashSet;
use std::path::PathBuf;

use super::Environment;

mod candidate;
mod config;
mod dedupe;
mod filesystem;
mod libvirt;

pub(crate) use candidate::{Discovered, ImageCandidate};
pub(crate) use config::parse_emu_dir;
pub(crate) use filesystem::{is_vm_image_path, legacy_root_image_count};

pub(crate) struct DiscoveryContext {
    pub(crate) config_path: Option<PathBuf>,
    pub(crate) home_dir: Option<PathBuf>,
    pub(crate) virsh: Option<PathBuf>,
    pub(crate) libvirt_uris: &'static [&'static str],
    pub(crate) emu_dir: PathBuf,
}

pub(crate) fn discover(context: DiscoveryContext) -> Result<Discovered> {
    let mut environments = Vec::new();
    environments.extend(config::discover(
        context.config_path.as_deref(),
        context.home_dir.as_ref(),
    )?);
    environments.extend(libvirt::discover(
        context.virsh.as_deref(),
        context.libvirt_uris,
    ));
    let environments = dedupe::dedupe_and_sort(environments);
    let mut seen = HashSet::new();
    let entries = filesystem::collect_image_paths(std::slice::from_ref(&context.emu_dir), &mut seen);
    Ok(Discovered::partition(environments, &entries))
}
```

**Verify**: this step plus Step 1 leave the crate non-building until the `emu.rs` callers are updated in Step 4. Defer the commit; the green gate is at the end of Step 5. Continue.

---

#### Step 3: Move the B1 `discover_scans_the_single_emu_dir` test onto the `Discovered` contract

Under the new contract, scanned `emu_dir` images are CANDIDATES, not environments. An unregistered `emu_dir` image lands in `.candidates`, and a config `[images.*]` env lands in `.environments`.

**Step 3a: Replace the B1 test** in the `#[cfg(test)] mod tests` block of `tools/qol-cli/src/commands/emu/discovery/mod.rs`.

**Before** (the B1 body):

```rust
    #[test]
    fn discover_scans_the_single_emu_dir() {
        let dir = std::env::temp_dir().join(format!("qol-emu-ctx-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("win11.qcow2"), b"x").unwrap();

        let environments = discover(DiscoveryContext {
            config_path: None,
            home_dir: None,
            virsh: None,
            libvirt_uris: &[],
            emu_dir: dir.clone(),
        })
        .unwrap();

        assert_eq!(environments.len(), 1, "environments: {environments:?}");
        assert_eq!(environments[0].source, "scan");

        let mut registered = HashSet::new();
        registered.insert(environments[0].image_path.clone());
        assert!(registered.iter().any(|p| p.ends_with("win11.qcow2")));
        fs::remove_dir_all(&dir).unwrap();
    }
```

**After**: assert on `Discovered`. An `[images.*]` config env is in `.environments`; the unregistered `emu_dir` image is in `.candidates`.

```rust
    #[test]
    fn discover_partitions_emu_dir_images_into_candidates() {
        let dir = std::env::temp_dir().join(format!("qol-emu-ctx-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let registered_file = dir.join("registered.qcow2");
        fs::write(&registered_file, b"x").unwrap();
        fs::write(dir.join("win11.qcow2"), b"x").unwrap();

        let config = dir.join("emu.toml");
        fs::write(
            &config,
            format!("[images]\nregistered = \"{}\"\n", registered_file.display()),
        )
        .unwrap();

        let discovered = discover(DiscoveryContext {
            config_path: Some(config),
            home_dir: None,
            virsh: None,
            libvirt_uris: &[],
            emu_dir: dir.clone(),
        })
        .unwrap();

        assert_eq!(
            discovered.environments.len(),
            1,
            "envs: {:?}",
            discovered.environments
        );
        assert_eq!(discovered.environments[0].id, "registered");
        assert_eq!(
            discovered.candidates.len(),
            1,
            "candidates: {:?}",
            discovered.candidates
        );
        assert!(discovered.candidates[0].path.ends_with("win11.qcow2"));
        fs::remove_dir_all(&dir).unwrap();
    }
```

The test module's `use std::collections::HashSet;` from B1 is no longer used here. If it is unused elsewhere in the module's tests, drop it so `cargo clippy --all-targets -- -D warnings` stays clean. Keep `use std::fs;` and `use super::*;`.

**Step 3b: Verify**: still not buildable until Step 4 fixes the `emu.rs` callers. Defer; continue.

---

#### Step 4: Re-export the candidate types and refactor `discover_environments`

Add the candidate types to the `emu.rs` discovery import line AND re-export them `pub(crate)`, so `commands::emu::Discovered` and `commands::emu::ImageCandidate` resolve crate-wide (the dev console names `ImageCandidate` through this path in B5). `discover_environments` becomes a thin wrapper around the new `discover_all`.

**Step 4a: Re-export the discovery types.** In `tools/qol-cli/src/commands/emu.rs`, B1 set the discovery import to `use discovery::{legacy_root_image_count, parse_emu_dir, DiscoveryContext};`. Promote it to a `pub(crate) use` that also brings in the two types, so they are re-exported from `emu.rs`:

```rust
pub(crate) use discovery::{
    legacy_root_image_count, parse_emu_dir, Discovered, DiscoveryContext, ImageCandidate,
};
```

A plain `use` would keep `discovery` private and `commands::emu::ImageCandidate` would not resolve from `dev_console.rs`; the `pub(crate) use` re-export is what makes it resolve. `legacy_root_image_count`/`parse_emu_dir`/`DiscoveryContext` were already used by B1 code, so promoting the line to `pub(crate)` introduces no unused-import.

**Step 4b: Add `discover_all` and rewrite `discover_environments`.**

**Before**:

```rust
fn discover_environments() -> Result<Vec<Environment>> {
    discovery::discover(DiscoveryContext {
        config_path: emu_config_path(),
        home_dir: dirs::home_dir(),
        virsh: find_on_path("virsh"),
        libvirt_uris: platform::libvirt_uris(),
        emu_dir: emu_dir().unwrap_or_default(),
    })
}
```

**After**: the `DiscoveryContext` build moves into `discover_all`, and `discover_environments` projects `.environments`:

```rust
pub(crate) fn discover_all() -> Result<Discovered> {
    discovery::discover(DiscoveryContext {
        config_path: emu_config_path(),
        home_dir: dirs::home_dir(),
        virsh: find_on_path("virsh"),
        libvirt_uris: platform::libvirt_uris(),
        emu_dir: emu_dir().unwrap_or_default(),
    })
}

fn discover_environments() -> Result<Vec<Environment>> {
    Ok(discover_all()?.environments)
}
```

`registered_image_paths`, `cmd_list`, `cmd_doctor`, and the legacy-count callers (all from B1) keep calling `discover_environments()` unchanged; they still receive `Vec<Environment>`. The `len()` caller in `cmd_doctor` and the empty/bail caller in `boot_vm` compile unchanged.

**Verify**: the crate builds once Step 5 lands the `emu_scan` producer (which is the non-test consumer of `ImageCandidate`). Continue to Step 5, then run the gate.

---

#### Step 5: Extract `statuses_for`, refactor `environment_statuses`, add the `emu_scan` producer

Extract the `last_runs_by_id` + map-to-`EnvironmentStatus` body from `environment_statuses` into `statuses_for(environments: Vec<Environment>) -> Vec<EnvironmentStatus>`, so both `environment_statuses` and the new `emu_scan` reuse it. `emu_scan` runs discovery ONCE via `discover_all` and returns both the statuses and the candidates.

**Step 5a: Write the failing test.** `statuses_for` is a pure transform, so it gets a unit test. Append to the top-level `#[cfg(test)] mod tests` in `tools/qol-cli/src/commands/emu.rs` (4-space indentation; `super::*` brings `Environment`, `GuestArch`, `Firmware`, `ResolveState`, `PathBuf` into scope):

```rust
    #[test]
    fn statuses_for_maps_each_environment_to_a_status() {
        let environments = vec![
            Environment {
                id: "alpha".to_string(),
                name: "Alpha".to_string(),
                backend: "qemu".to_string(),
                arch: GuestArch::X86_64,
                image_path: PathBuf::from("/a/b/alpha.qcow2"),
                source: "config".to_string(),
                firmware: Firmware::Bios,
            },
            Environment {
                id: "beta".to_string(),
                name: "Beta".to_string(),
                backend: "qemu".to_string(),
                arch: GuestArch::Aarch64,
                image_path: PathBuf::from("/a/b/beta.qcow2"),
                source: "config".to_string(),
                firmware: Firmware::Uefi,
            },
        ];
        let statuses = statuses_for(environments);
        let ids: Vec<&str> = statuses.iter().map(|status| status.id.as_str()).collect();
        assert_eq!(ids, vec!["alpha", "beta"], "statuses: {statuses:?}");
        assert!(statuses.iter().all(|status| status.backend == "qemu"));
    }
```

The `firmware:` field on `Environment` is the B3 addition; this test is written for the post-B3 sequence in which B2e runs.

**Step 5b: Run test to verify it fails.**

```bash
cargo test -p qol --bin qol commands::emu::tests::statuses_for_maps_each_environment_to_a_status 2>&1 | tail -20
```

Expected: FAIL, `cannot find function statuses_for in this scope`.

**Step 5c: Write minimal implementation.** In `tools/qol-cli/src/commands/emu.rs`, extract `statuses_for`, reduce `environment_statuses` to a wrapper, and add `emu_scan`.

**Before** (`environment_statuses`):

```rust
pub(crate) fn environment_statuses() -> Result<Vec<EnvironmentStatus>> {
    let mut last_runs = last_runs_by_id();
    Ok(discover_environments()?
        .into_iter()
        .map(|environment| {
            let resolution = resolve_environment(&environment);
            EnvironmentStatus {
                last_run: last_runs.remove(&environment.id),
                id: environment.id,
                backend: environment.backend,
                state: resolution.state,
                reason: resolution.reason,
            }
        })
        .collect())
}
```

**After**:

```rust
fn statuses_for(environments: Vec<Environment>) -> Vec<EnvironmentStatus> {
    let mut last_runs = last_runs_by_id();
    environments
        .into_iter()
        .map(|environment| {
            let resolution = resolve_environment(&environment);
            EnvironmentStatus {
                last_run: last_runs.remove(&environment.id),
                id: environment.id,
                backend: environment.backend,
                state: resolution.state,
                reason: resolution.reason,
            }
        })
        .collect()
}

pub(crate) fn environment_statuses() -> Result<Vec<EnvironmentStatus>> {
    Ok(statuses_for(discover_environments()?))
}

pub(crate) fn emu_scan() -> Result<(Vec<EnvironmentStatus>, Vec<ImageCandidate>)> {
    let discovered = discover_all()?;
    Ok((statuses_for(discovered.environments), discovered.candidates))
}
```

`statuses_for` is byte-for-byte the old body, so `environment_statuses` behaves identically. `emu_scan` runs `discover_all` once: `statuses_for` consumes `discovered.environments`, and `discovered.candidates` is returned alongside. This is the production consumer of `ImageCandidate` that keeps the `emu.rs` re-export and the `Discovered`/`ImageCandidate` types reachable under `-D warnings`.

**Step 5d: Run test to verify it passes, build, clippy.**

```bash
cargo build -p qol 2>&1 | tail -20
cargo test -p qol --bin qol commands::emu 2>&1 | tail -20
cargo clippy -p qol --all-targets -- -D warnings 2>&1 | tail -30
```

Expected: PASS. `statuses_for_maps_each_environment_to_a_status` green, `discover_partitions_emu_dir_images_into_candidates` green, the four `EmuState::Done(vec![...])` dev-console tests and `emu_status` untouched and green. Clippy clean: no `unused_imports` on the discovery re-exports (`emu_scan` consumes `ImageCandidate`, `discover_all` consumes `Discovered`), no `dead_code` (the filesystem `Environment` producer is gone, so nothing references the dropped `humanize_id`/`sanitize_id` through it).

**Step 5e: Commit.** One commit lands the whole rewire (Steps 1-5 do not compile in isolation; this is the first green state):

```bash
git add tools/qol-cli/src/commands/emu/discovery/filesystem.rs tools/qol-cli/src/commands/emu/discovery/mod.rs tools/qol-cli/src/commands/emu.rs
git commit -m "feat(emu): produce candidates from discovery via Discovered::partition"
```


### Task B3: qemu-img validate + toml_edit register + parser/firmware widening + Environment.firmware

Preconditions from earlier steps that this task treats as pre-existing:
- B2 has introduced `pub(crate) enum Firmware { Bios, Uefi }` (derives `Clone, Copy, Debug, PartialEq, Eq`) with `Firmware::as_str(self) -> &'static str` ("bios"/"uefi"), `Firmware::parse(&str) -> Option<Firmware>`, and `Firmware::for_arch(GuestArch) -> Firmware` (`Aarch64 -> Uefi`, `X86_64 -> Bios`), plus `pub(crate) struct ImageCandidate` (derives `Clone, Debug, PartialEq, Eq`) with fields `id: String, path: PathBuf, display_name: String, arch: GuestArch, arch_inferred: bool, firmware: Firmware`. `Firmware` lives in `arch.rs` (re-exported from `emu.rs`); `ImageCandidate` lives in `discovery/candidate.rs` (re-exported from `discovery/mod.rs`). The derived `Debug`/`PartialEq` impls read every `ImageCandidate` field, so the struct does not trip `field is never read` under `-D warnings` before B5 wires its rendering.

This task is split into six commits, each compiling and green:
1. Declare `toml_edit = "0.25"` (root workspace + qol-cli).
2. Add `Environment.firmware` field and fix the five literals + report_json.
3. Widen `parse_image_overrides` to `(PathBuf, GuestArch, Firmware)`, promote to `pub(crate)`, read optional `firmware`.
4. Add arch/firmware filename-inference helpers + `infer_candidate`.
5. Add `qemu-img info --output=json` parsing.
6. Add `register_image` (validate + toml_edit write with parse-first dedup).

#### Files
- Modify: `/Users/kaho/repos/private/qol-monorepo/Cargo.toml` (line 31, `[workspace.dependencies]`)
- Modify: `/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/Cargo.toml` (lines 13-20, `[dependencies]`)
- Modify: `/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs` (Environment struct lines 28-36; report_json lines 920-956; test literals lines 1059-1066 and 1099-1106)
- Modify: `/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/config.rs` (whole file)
- Modify: `/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/libvirt.rs` (Environment literal lines 19-26)
- Modify: `/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/filesystem.rs` (Environment literal lines 44-51; add `infer_candidate`)
- Modify: `/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/arch.rs` (add inference helpers + tests)
- Create: `/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/registry.rs` (new module: qemu-img parse + register_image)
- Modify: `/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs` (`mod registry;`)
- Test: inline `#[cfg(test)]` modules in `config.rs`, `arch.rs`, `registry.rs`, and `emu.rs`

---

#### Commit 1 - Declare `toml_edit = "0.25"` (dependency-only, no unit test)

- [ ] Step 1: Add `toml_edit` to root `[workspace.dependencies]`. Edit `/Users/kaho/repos/private/qol-monorepo/Cargo.toml` and change the `toml = "0.9"` line region so it reads:

```toml
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
toml = "0.9"
toml_edit = "0.25"
x11rb = "0.13"
```

- [ ] Step 2: Add the workspace dep to qol-cli. Edit `/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/Cargo.toml` `[dependencies]` so it reads:

```toml
[dependencies]
anyhow = "1.0"
dirs = "6.0"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
toml.workspace = true
toml_edit.workspace = true
ratatui = "0.30"
ansi-to-tui = "8.0"
```

> NOTE: A3a already added `qol-config.workspace = true` to this `[dependencies]` block. Preserve that line; the edit above only adds `toml_edit.workspace = true`. The final block contains both `qol-config.workspace = true` (from A3a) and `toml_edit.workspace = true` (this commit).

- [ ] Step 3: Build to verify resolution.

```bash
cargo build -p qol
```

Expected: PASS (compiles; `toml_edit` 0.25.12+spec-1.1.0 resolves - already present in Cargo.lock as a transitive dep - and shares the `spec-1.1.0` substrate with `toml 0.9`).

- [ ] Step 4: Commit.

```bash
git add Cargo.toml Cargo.lock tools/qol-cli/Cargo.toml
git commit -m "build(qol): declare toml_edit 0.25 for emu registry writes"
```

---

#### Commit 2 - Add `Environment.firmware` field + fix all five literals + report_json

- [ ] Step 1: Write the failing test. Append this test into the `#[cfg(test)] mod tests` in `/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs` (after `sanitizes_domain_names_for_cli_ids`):

```rust
    #[test]
    fn report_json_serializes_firmware() {
        let environment = Environment {
            id: "foo".to_string(),
            name: "Foo".to_string(),
            backend: "qemu".to_string(),
            arch: GuestArch::X86_64,
            image_path: PathBuf::from("/a/b/base.qcow2"),
            source: "config".to_string(),
            firmware: Firmware::Uefi,
        };
        let resolution = Resolution {
            state: ResolveState::Ready,
            reason: "ready".to_string(),
            image_path: PathBuf::from("/a/b/base.qcow2"),
            qemu_system: None,
            qemu_img: None,
            acceleration: "kvm",
            firmware: None,
        };
        let report = report_json(ReportInput {
            environment: &environment,
            resolution: &resolution,
            run_dir: Path::new("/a/b/run"),
            status: "ok",
            overlay: None,
            qemu_command: None,
            commands: Vec::new(),
            qmp: None,
            serial: None,
            workflow: None,
            teardown: None,
            next: Vec::new(),
            started_at: 0,
        })
        .unwrap();
        assert_eq!(report["environment"]["firmware"], "uefi");
    }
```

- [ ] Step 2: Run test to verify it fails.

```bash
cargo test -p qol --lib commands::emu::tests::report_json_serializes_firmware
```

Expected: FAIL to compile - `Environment` has no field `firmware`, and `report_json` does not emit a `firmware` key.

- [ ] Step 3a: Add the field to `Environment`. Edit the struct at `emu.rs` lines 28-36 to:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Environment {
    id: String,
    name: String,
    backend: String,
    arch: GuestArch,
    image_path: PathBuf,
    source: String,
    firmware: Firmware,
}
```

- [ ] Step 3b: Make `Firmware` reachable in `emu.rs`. B2 already re-exported it at line 25 (`pub(crate) use arch::{Firmware, GuestArch};`), so `Firmware` is in scope in `emu.rs` with no further import. Do NOT add a second `use discovery::Firmware;` - that would be a duplicate-import error.

- [ ] Step 3c: Serialize firmware in `report_json`. Edit the `"environment"` object in `report_json` (emu.rs lines 927-934) to add the `firmware` line:

```rust
        "environment": {
            "id": input.environment.id,
            "name": input.environment.name,
            "backend": input.environment.backend,
            "arch": input.environment.arch.as_str(),
            "image_path": input.environment.image_path,
            "source": input.environment.source,
            "firmware": input.environment.firmware.as_str(),
        },
```

- [ ] Step 3d: Fix the libvirt literal (arch-derived default). Edit `/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/libvirt.rs` lines 19-26 to:

```rust
            environments.push(Environment {
                id: sanitize_id(&domain),
                name: domain,
                backend: "qemu".to_string(),
                arch: GuestArch::X86_64,
                image_path,
                source: format!("libvirt:{uri}"),
                firmware: Firmware::for_arch(GuestArch::X86_64),
            });
```

Add `Firmware` to the `use super::super::{...}` import at libvirt.rs line 4:

```rust
use super::super::{arch::GuestArch, sanitize_id, Environment, Firmware};
```

- [ ] Step 3e: Fix the filesystem literal. Edit `/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/filesystem.rs` lines 44-51 to:

```rust
        environments.push(Environment {
            name: humanize_id(&id),
            id,
            backend: "qemu".to_string(),
            arch: GuestArch::X86_64,
            image_path: canonical,
            source: "scan".to_string(),
            firmware: Firmware::for_arch(GuestArch::X86_64),
        });
```

Add `Firmware` to the `use super::super::{...}` import at filesystem.rs line 5:

```rust
use super::super::{arch::GuestArch, humanize_id, sanitize_id, Environment, Firmware};
```

> NOTE: B1 Part 1 already moved this literal into `collect_image_environments`. Apply this firmware field to wherever the filesystem `Environment` literal now lives after B1 (the `collect_image_environments` map closure), not necessarily lines 44-51. The field set is identical; only its line location moved.

- [ ] Step 3f: Fix the two emu.rs test literals. In `qemu_args_wire_accel_display_and_qmp` (lines 1059-1066) add `firmware: Firmware::Bios,` as the last field:

```rust
        let environment = Environment {
            id: "foo".to_string(),
            name: "Foo".to_string(),
            backend: "qemu".to_string(),
            arch: GuestArch::X86_64,
            image_path: PathBuf::from("/a/b/base.qcow2"),
            source: "config".to_string(),
            firmware: Firmware::Bios,
        };
```

In `qemu_args_wire_aarch64_machine_cpu_and_firmware` (lines 1099-1106) add `firmware: Firmware::Uefi,` as the last field:

```rust
        let environment = Environment {
            id: "foo".to_string(),
            name: "Foo".to_string(),
            backend: "qemu".to_string(),
            arch: GuestArch::Aarch64,
            image_path: PathBuf::from("/a/b/base.qcow2"),
            source: "config".to_string(),
            firmware: Firmware::Uefi,
        };
```

- [ ] Step 3g: Fix the config.rs literal. Edit `/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/config.rs` `discover` (lines 9-21) so the mapped tuple carries firmware. This step keeps `parse_image_overrides` 2-tuple shape for now (Commit 3 widens it); supply the arch default to keep it compiling:

```rust
pub(crate) fn discover(path: Option<&Path>, home: Option<&PathBuf>) -> Result<Vec<Environment>> {
    Ok(load_image_overrides(path, home)?
        .into_iter()
        .map(|(id, (image_path, arch))| Environment {
            name: humanize_id(&id),
            id,
            backend: "qemu".to_string(),
            arch,
            image_path,
            source: "config".to_string(),
            firmware: Firmware::for_arch(arch),
        })
        .collect())
}
```

Add `Firmware` to the config.rs import at line 7:

```rust
use super::super::{arch::GuestArch, humanize_id, sanitize_id, Environment, Firmware};
```

- [ ] Step 4: Run test to verify it passes.

```bash
cargo test -p qol --lib commands::emu::tests::report_json_serializes_firmware
```

Expected: PASS.

- [ ] Step 5: Verify the whole crate still builds and existing emu tests pass.

```bash
cargo test -p qol --lib commands::emu && cargo clippy -p qol --all-targets -- -D warnings
```

Expected: PASS.

- [ ] Step 6: Commit.

```bash
git add tools/qol-cli/src/commands/emu.rs tools/qol-cli/src/commands/emu/discovery/config.rs tools/qol-cli/src/commands/emu/discovery/libvirt.rs tools/qol-cli/src/commands/emu/discovery/filesystem.rs
git commit -m "feat(emu): carry firmware through Environment and report.json"
```

---

#### Commit 3 - Promote + widen `parse_image_overrides` to `(PathBuf, GuestArch, Firmware)` reading optional `firmware`

- [ ] Step 1: Write the failing tests. Replace the four existing tests in the `#[cfg(test)] mod tests` of `config.rs` with widened destructuring and add firmware tests. The full replacement test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_image_overrides_with_sanitized_ids() {
        let home = PathBuf::from("/home/me");
        let overrides = parse_image_overrides(
            r#"
[images]
"Windows 11" = "~/vm/windows.qcow2"
"#,
            Some(&home),
        )
        .unwrap();
        let (path, arch, firmware) = overrides.get("windows-11").unwrap();
        assert_eq!(path, &PathBuf::from("/home/me/vm/windows.qcow2"));
        assert_eq!(*arch, GuestArch::X86_64);
        assert_eq!(*firmware, Firmware::Bios);
    }

    #[test]
    fn parses_table_form_with_arch() {
        let overrides = parse_image_overrides(
            r#"
[images.foo]
path = "/a/b/foo.qcow2"
arch = "aarch64"
"#,
            None,
        )
        .unwrap();
        let (path, arch, firmware) = overrides.get("foo").unwrap();
        assert_eq!(path, &PathBuf::from("/a/b/foo.qcow2"));
        assert_eq!(*arch, GuestArch::Aarch64);
        assert_eq!(*firmware, Firmware::Uefi);
    }

    #[test]
    fn firmware_defaults_per_arch_and_reads_explicit() {
        let cases = [
            ("path = \"/a/x.qcow2\"\narch = \"x86_64\"", Firmware::Bios),
            ("path = \"/a/x.qcow2\"\narch = \"aarch64\"", Firmware::Uefi),
            (
                "path = \"/a/x.qcow2\"\narch = \"x86_64\"\nfirmware = \"uefi\"",
                Firmware::Uefi,
            ),
            (
                "path = \"/a/x.qcow2\"\narch = \"aarch64\"\nfirmware = \"bios\"",
                Firmware::Bios,
            ),
        ];
        for (body, expected) in cases {
            let content = format!("[images.foo]\n{body}\n");
            let overrides = parse_image_overrides(&content, None).unwrap();
            let (_, _, firmware) = overrides.get("foo").unwrap();
            assert_eq!(*firmware, expected, "body: {body}");
        }
    }

    #[test]
    fn rejects_unknown_arch() {
        let error = parse_image_overrides(
            r#"
[images.foo]
path = "/a/b/foo.qcow2"
arch = "sparc"
"#,
            None,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("images.foo.arch"),
            "error: {error}"
        );
    }

    #[test]
    fn rejects_unknown_firmware() {
        let error = parse_image_overrides(
            r#"
[images.foo]
path = "/a/b/foo.qcow2"
firmware = "coreboot"
"#,
            None,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("images.foo.firmware"),
            "error: {error}"
        );
    }

    #[test]
    fn rejects_non_string_non_table_entries() {
        let error = parse_image_overrides(
            r#"
[images]
foo = 42
"#,
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("images.foo"), "error: {error}");
    }
}
```

> NOTE: B1 Part 3 added `parses_top_level_dir_with_home_expansion` to this same test module. Preserve that test when replacing the four override tests above - append it back, or scope the replacement to the four override tests and leave the dir test untouched.

- [ ] Step 2: Run test to verify it fails.

```bash
cargo test -p qol --lib commands::emu::discovery::config
```

Expected: FAIL to compile - `parse_image_overrides` still returns a 2-tuple and does not read `firmware`.

- [ ] Step 3: Widen and promote `parse_image_overrides`, update `load_image_overrides` return type, and simplify `discover`'s map. Replace `config.rs` lines 9-69 (the `discover`, `load_image_overrides`, and `parse_image_overrides` functions) with the following. Note the `TomlValue::String` arm is written as a single expression (no block braces) so `cargo fmt --check` stays clean:

```rust
pub(crate) fn discover(path: Option<&Path>, home: Option<&PathBuf>) -> Result<Vec<Environment>> {
    Ok(load_image_overrides(path, home)?
        .into_iter()
        .map(|(id, (image_path, arch, firmware))| Environment {
            name: humanize_id(&id),
            id,
            backend: "qemu".to_string(),
            arch,
            image_path,
            source: "config".to_string(),
            firmware,
        })
        .collect())
}

fn load_image_overrides(
    path: Option<&Path>,
    home: Option<&PathBuf>,
) -> Result<HashMap<String, (PathBuf, GuestArch, Firmware)>> {
    let Some(path) = path else {
        return Ok(HashMap::new());
    };
    if !path.is_file() {
        return Ok(HashMap::new());
    }
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    parse_image_overrides(&content, home)
        .with_context(|| format!("failed to parse {}", path.display()))
}

pub(crate) fn parse_image_overrides(
    content: &str,
    home: Option<&PathBuf>,
) -> Result<HashMap<String, (PathBuf, GuestArch, Firmware)>> {
    let parsed: TomlValue = toml::from_str(content).context("invalid emu config TOML")?;
    let Some(images) = parsed.get("images").and_then(TomlValue::as_table) else {
        return Ok(HashMap::new());
    };
    let mut overrides = HashMap::new();
    for (id, value) in images {
        let (path, arch, firmware) = match value {
            TomlValue::String(path) => (path.as_str(), GuestArch::X86_64, Firmware::Bios),
            TomlValue::Table(table) => {
                let path = table
                    .get("path")
                    .and_then(TomlValue::as_str)
                    .ok_or_else(|| anyhow!("images.{id}.path must be a string path"))?;
                let arch = match table.get("arch") {
                    None => GuestArch::X86_64,
                    Some(value) => value.as_str().and_then(GuestArch::parse).ok_or_else(|| {
                        anyhow!("images.{id}.arch must be one of: x86_64, aarch64")
                    })?,
                };
                let firmware = match table.get("firmware") {
                    None => Firmware::for_arch(arch),
                    Some(value) => value.as_str().and_then(Firmware::parse).ok_or_else(|| {
                        anyhow!("images.{id}.firmware must be one of: bios, uefi")
                    })?,
                };
                (path, arch, firmware)
            }
            _ => bail!("images.{id} must be a string path or a table with path/arch"),
        };
        overrides.insert(sanitize_id(id), (expand_home(path, home), arch, firmware));
    }
    Ok(overrides)
}
```

- [ ] Step 4: Run test to verify it passes.

```bash
cargo test -p qol --lib commands::emu::discovery::config
```

Expected: PASS (all config tests, including the two new firmware tests and B1's dir test).

- [ ] Step 5: Commit.

```bash
git add tools/qol-cli/src/commands/emu/discovery/config.rs
git commit -m "feat(emu): widen parse_image_overrides to carry firmware"
```

---

#### Commit 4 - Arch/firmware filename-inference helpers + `infer_candidate`

- [ ] Step 1: Write the failing tests. Append into the `#[cfg(test)] mod tests` in `/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/arch.rs`:

```rust
    #[test]
    fn infers_arch_from_filename_tokens() {
        let cases = [
            ("ubuntu-arm64.qcow2", Some(GuestArch::Aarch64)),
            ("debian-aarch64.img", Some(GuestArch::Aarch64)),
            ("win11-amd64.vhdx", Some(GuestArch::X86_64)),
            ("fedora-x86_64.raw", Some(GuestArch::X86_64)),
            ("disk-x64.qcow2", Some(GuestArch::X86_64)),
            ("legacy-i386.img", Some(GuestArch::X86_64)),
            ("old-i686.img", Some(GuestArch::X86_64)),
            ("mystery.qcow2", None),
        ];
        for (name, expected) in cases {
            assert_eq!(infer_arch_from_filename(name), expected, "name: {name}");
        }
    }

    #[test]
    fn detects_windows_image_hint() {
        let cases = [
            ("win11.qcow2", true),
            ("windows-server.img", true),
            ("disk.vhdx", true),
            ("ubuntu.qcow2", false),
        ];
        for (name, expected) in cases {
            assert_eq!(is_windows_image_hint(name), expected, "name: {name}");
        }
    }

    #[test]
    fn infers_firmware_per_arch_and_windows_hint() {
        let cases = [
            (GuestArch::Aarch64, "ubuntu-arm64.qcow2", Firmware::Uefi),
            (GuestArch::X86_64, "ubuntu.qcow2", Firmware::Bios),
            (GuestArch::X86_64, "win11.vhdx", Firmware::Uefi),
            (GuestArch::X86_64, "windows-server.qcow2", Firmware::Uefi),
        ];
        for (arch, name, expected) in cases {
            assert_eq!(infer_firmware(arch, name), expected, "name: {name}");
        }
    }
```

- [ ] Step 2: Run test to verify it fails.

```bash
cargo test -p qol --lib commands::emu::arch
```

Expected: FAIL to compile - `infer_arch_from_filename`, `is_windows_image_hint`, `infer_firmware` are undefined.

- [ ] Step 3: Add the inference helpers as free functions in `arch.rs` (after the `impl Firmware` block, before the `#[cfg(test)]` module). `Firmware` is already in scope in `arch.rs` (B2 defined it there). Three points are load-bearing: (1) the boolean chain in `infer_arch_from_filename` is written one-condition-per-line exactly as rustfmt emits it, so `cargo fmt --check` stays clean; (2) `host_native_arch` is intentionally NOT added here - it has no caller in B3 and would be flagged as dead code under `-D warnings`; (3) `infer_arch_from_filename`/`is_windows_image_hint`/`infer_firmware` each carry `#[allow(dead_code)]` because B3's only callers are the `#[cfg(test)]` tests above plus `infer_candidate` (which is itself `#[allow(dead_code)]` until B5 wires it); `cargo clippy --all-targets -- -D warnings` compiles the lib target without `cfg(test)`, where a test-only callee is dead:

```rust
#[allow(dead_code)]
pub(crate) fn infer_arch_from_filename(name: &str) -> Option<GuestArch> {
    let lower = name.to_ascii_lowercase();
    let contains = |needle: &str| lower.contains(needle);
    if contains("arm64") || contains("aarch64") {
        return Some(GuestArch::Aarch64);
    }
    if contains("amd64")
        || contains("x86_64")
        || contains("x64")
        || contains("i386")
        || contains("i686")
    {
        return Some(GuestArch::X86_64);
    }
    None
}

#[allow(dead_code)]
pub(crate) fn is_windows_image_hint(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("win") || lower.contains("windows") || lower.ends_with(".vhdx")
}

#[allow(dead_code)]
pub(crate) fn infer_firmware(arch: GuestArch, name: &str) -> Firmware {
    match arch {
        GuestArch::Aarch64 => Firmware::Uefi,
        GuestArch::X86_64 => {
            if is_windows_image_hint(name) {
                Firmware::Uefi
            } else {
                Firmware::Bios
            }
        }
    }
}
```

- [ ] Step 4: Write the failing test for `infer_candidate`. Append to the `#[cfg(test)] mod tests` in `tools/qol-cli/src/commands/emu/discovery/filesystem.rs`:

```rust
    #[test]
    fn infer_candidate_fills_arch_firmware_and_id_from_filename() {
        let root = std::env::temp_dir().join(format!("qol-emu-infer-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let image = root.join("win11-arm64.qcow2");
        fs::write(&image, b"x").unwrap();

        let candidate = infer_candidate(&image);

        assert_eq!(candidate.arch, GuestArch::Aarch64, "arm64 token");
        assert!(candidate.arch_inferred, "arch was inferred from filename");
        assert_eq!(candidate.firmware, Firmware::Uefi, "arm => uefi");
        assert_eq!(candidate.id, "win11-arm64");
        assert_eq!(candidate.path, image.canonicalize().unwrap());

        fs::remove_dir_all(&root).unwrap();
    }
```

- [ ] Step 5: Run the test to verify it fails.

```bash
cargo test -p qol --lib commands::emu::discovery::filesystem::tests::infer_candidate_fills_arch_firmware_and_id_from_filename
```

Expected: FAIL to compile - `infer_candidate` is undefined.

- [ ] Step 6: Add `infer_candidate` to `tools/qol-cli/src/commands/emu/discovery/filesystem.rs` (after `image_id`, before the test module). It builds an `ImageCandidate` from a path: canonicalizes, derives the id via `image_id`, and applies filename arch/firmware inference (falling back to `GuestArch::X86_64` + `Firmware::for_arch` when arch is not inferable). Bring the inference helpers and `ImageCandidate`/`Firmware` into scope. The `#[allow(dead_code)]` stays until B5 wires the command caller:

```rust
use super::super::arch::{infer_arch_from_filename, infer_firmware, Firmware};
use super::candidate::ImageCandidate;

#[allow(dead_code)]
pub(crate) fn infer_candidate(path: &Path) -> ImageCandidate {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let id = image_id(&canonical);
    let display_name = humanize_id(&id);
    let name = canonical
        .file_name()
        .and_then(|os| os.to_str())
        .unwrap_or_default();
    let inferred = infer_arch_from_filename(name);
    let arch = inferred.unwrap_or(GuestArch::X86_64);
    let firmware = infer_firmware(arch, name);
    ImageCandidate {
        id,
        path: canonical,
        display_name,
        arch,
        arch_inferred: inferred.is_some(),
        firmware,
    }
}
```

> NOTE on visibility: `ImageCandidate`'s fields are `pub(crate)` (B2), and `filesystem` is a sibling of `candidate` under `discovery`, so `super::candidate::ImageCandidate` and its fields are reachable. Re-export `infer_candidate` from `discovery/mod.rs` so B5 can call `discovery::infer_candidate`:
>
> ```rust
> pub(crate) use filesystem::{infer_candidate, is_vm_image_path, legacy_root_image_count};
> ```
>
> Adjust the existing `pub(crate) use filesystem::{...}` line (B1 set it to `{is_vm_image_path, legacy_root_image_count}`) to add `infer_candidate`.

- [ ] Step 7: Run both tests to verify they pass, and confirm the clippy gate is clean (the `-D warnings` run is what proves the `#[allow(dead_code)]` markers are correct).

```bash
cargo test -p qol --lib commands::emu::arch && \
cargo test -p qol --lib commands::emu::discovery::filesystem::tests::infer_candidate_fills_arch_firmware_and_id_from_filename && \
cargo clippy -p qol --all-targets -- -D warnings
```

Expected: PASS.

- [ ] Step 8: Commit.

```bash
git add tools/qol-cli/src/commands/emu/arch.rs tools/qol-cli/src/commands/emu/discovery/filesystem.rs tools/qol-cli/src/commands/emu/discovery/mod.rs
git commit -m "feat(emu): infer arch/firmware from filenames and build ImageCandidate"
```

---

#### Commit 5 - `qemu-img info --output=json` parsing (new `registry.rs`)

- [ ] Step 1: Declare the module. In `/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs`, add `mod registry;` alongside the existing emu submodule declarations (the `mod arch; ... mod workflow;` block at lines 14-23). Then create `/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/registry.rs` with the failing test only. `QemuImgInfo` and `parse_qemu_img_info` carry `#[allow(dead_code)]` for this commit because their only caller is `#[cfg(test)]` here; the allow is removed in Commit 6 once `register_image` references them:

```rust
use anyhow::{anyhow, Result};

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct QemuImgInfo {
    pub(crate) format: String,
    pub(crate) virtual_size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_qemu_img_info_json() {
        let json = r#"{"virtual-size":21474836480,"filename":"/a/b/x.qcow2","format":"qcow2","actual-size":1234}"#;
        let info = parse_qemu_img_info(json).unwrap();
        assert_eq!(info.format, "qcow2");
        assert_eq!(info.virtual_size, 21474836480);
    }

    #[test]
    fn rejects_missing_format() {
        let json = r#"{"virtual-size":1024}"#;
        let error = parse_qemu_img_info(json).unwrap_err();
        assert!(error.to_string().contains("format"), "error: {error}");
    }

    #[test]
    fn rejects_unknown_format() {
        let json = r#"{"format":"mystery","virtual-size":1024}"#;
        let error = parse_qemu_img_info(json).unwrap_err();
        assert!(
            error.to_string().contains("unknown image format"),
            "error: {error}"
        );
    }
}
```

- [ ] Step 2: Run test to verify it fails.

```bash
cargo test -p qol --lib commands::emu::registry
```

Expected: FAIL to compile - `parse_qemu_img_info` is undefined.

- [ ] Step 3: Add `parse_qemu_img_info` to `registry.rs` (above the `#[cfg(test)]` module), carrying `#[allow(dead_code)]` for the same reason as the struct:

```rust
const KNOWN_FORMATS: &[&str] = &["qcow2", "qcow", "raw", "vhd", "vhdx", "vmdk", "vpc"];

#[allow(dead_code)]
pub(crate) fn parse_qemu_img_info(json: &str) -> Result<QemuImgInfo> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| anyhow!("invalid qemu-img JSON: {e}"))?;
    let format = value
        .get("format")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("qemu-img info missing `format`"))?
        .to_string();
    if !KNOWN_FORMATS.contains(&format.as_str()) {
        return Err(anyhow!("unknown image format `{format}`"));
    }
    let virtual_size = value
        .get("virtual-size")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| anyhow!("qemu-img info missing `virtual-size`"))?;
    Ok(QemuImgInfo {
        format,
        virtual_size,
    })
}
```

- [ ] Step 4: Run test to verify it passes, and confirm the clippy gate is clean.

```bash
cargo test -p qol --lib commands::emu::registry && cargo clippy -p qol --all-targets -- -D warnings
```

Expected: PASS.

- [ ] Step 5: Commit.

```bash
git add tools/qol-cli/src/commands/emu.rs tools/qol-cli/src/commands/emu/registry.rs
git commit -m "feat(emu): parse qemu-img info json with format validation"
```

---

#### Commit 6 - `register_image` (validate via qemu-img, then toml_edit write with parse-first dedup)

- [ ] Step 1: Write the failing tests. Append to the `#[cfg(test)] mod tests` in `registry.rs`. These tests exercise only the write path (`write_image_entry`), which takes a pre-parsed `ImageCandidate` so no real `qemu-img` binary is needed; the `qemu-img` orchestration in `register_image` is a thin wrapper not unit-tested per the no-thin-wrapper rule. The two long lines (the `format!` in `relative_or_symlinked_image_path_still_dedups` and the final `assert!` in `fails_on_malformed_toml_without_appending`) are pre-wrapped to rustfmt's canonical multi-line layout so `cargo fmt --check` stays clean:

```rust
    use crate::commands::emu::arch::GuestArch;
    use crate::commands::emu::discovery::Firmware;
    use crate::commands::emu::discovery::ImageCandidate;
    use tempfile::tempdir;

    fn candidate(dir: &std::path::Path, file: &str, arch: GuestArch, fw: Firmware) -> ImageCandidate {
        let path = dir.join(file);
        std::fs::write(&path, b"img").unwrap();
        ImageCandidate {
            id: "win11".to_string(),
            path,
            display_name: "Win11".to_string(),
            arch,
            arch_inferred: true,
            firmware: fw,
        }
    }

    #[test]
    fn writes_image_table_preserving_dir_and_comments() {
        let dir = tempdir().unwrap();
        let emu_toml = dir.path().join("emu.toml");
        std::fs::write(
            &emu_toml,
            "# my emus\ndir = \"~/vms\"\n\n[images.existing]\npath = \"/a/old.qcow2\"\n",
        )
        .unwrap();
        let cand = candidate(dir.path(), "win11.qcow2", GuestArch::X86_64, Firmware::Uefi);
        let id = write_image_entry(&emu_toml, &cand).unwrap();
        assert_eq!(id, "win11");
        let written = std::fs::read_to_string(&emu_toml).unwrap();
        assert!(written.contains("# my emus"), "comment dropped: {written}");
        assert!(written.contains("dir = \"~/vms\""), "dir dropped: {written}");
        assert!(written.contains("[images.win11]"), "table missing: {written}");
        assert!(written.contains("arch = \"x86_64\""), "arch missing: {written}");
        assert!(
            written.contains("firmware = \"uefi\""),
            "firmware missing: {written}"
        );
    }

    #[test]
    fn skips_when_id_already_registered_by_canonical_path() {
        let dir = tempdir().unwrap();
        let emu_toml = dir.path().join("emu.toml");
        let cand = candidate(dir.path(), "win11.qcow2", GuestArch::X86_64, Firmware::Bios);
        let canonical = cand.path.canonicalize().unwrap();
        std::fs::write(
            &emu_toml,
            format!("[images.win11]\npath = \"{}\"\n", canonical.display()),
        )
        .unwrap();
        let before = std::fs::read_to_string(&emu_toml).unwrap();
        let id = write_image_entry(&emu_toml, &cand).unwrap();
        assert_eq!(id, "win11");
        let after = std::fs::read_to_string(&emu_toml).unwrap();
        assert_eq!(before, after, "must not append a duplicate entry");
    }

    #[test]
    fn fails_on_malformed_toml_without_appending() {
        let dir = tempdir().unwrap();
        let emu_toml = dir.path().join("emu.toml");
        std::fs::write(&emu_toml, "this is = = not valid toml\n").unwrap();
        let cand = candidate(dir.path(), "win11.qcow2", GuestArch::X86_64, Firmware::Bios);
        let error = write_image_entry(&emu_toml, &cand).unwrap_err();
        assert!(
            error.to_string().contains("emu.toml") || error.to_string().contains("parse"),
            "error: {error}"
        );
        let after = std::fs::read_to_string(&emu_toml).unwrap();
        assert!(
            !after.contains("[images.win11]"),
            "appended on malformed: {after}"
        );
    }

    #[test]
    fn relative_or_symlinked_image_path_still_dedups() {
        let dir = tempdir().unwrap();
        let emu_toml = dir.path().join("emu.toml");
        let cand = candidate(dir.path(), "win11.qcow2", GuestArch::X86_64, Firmware::Bios);
        let canonical = cand.path.canonicalize().unwrap();
        std::fs::write(
            &emu_toml,
            format!(
                "[images.other]\npath = \"{}/./win11.qcow2\"\n",
                canonical.parent().unwrap().display()
            ),
        )
        .unwrap();
        let id = write_image_entry(&emu_toml, &cand).unwrap();
        assert_eq!(id, "win11");
        let after = std::fs::read_to_string(&emu_toml).unwrap();
        assert!(
            !after.contains("[images.win11]"),
            "canonical-equal path should dedup, got: {after}"
        );
    }
```

- [ ] Step 2: Run test to verify it fails.

```bash
cargo test -p qol --lib commands::emu::registry
```

Expected: FAIL to compile - `write_image_entry` and `register_image` are undefined.

- [ ] Step 3: Implement `register_image` and `write_image_entry` in `registry.rs`. First, remove the now-redundant `#[allow(dead_code)]` from `QemuImgInfo` and `parse_qemu_img_info` (added in Commit 5) - they become live because `register_image` references them. Then change the top-of-file import line from `use anyhow::{anyhow, Result};` to the block below, and add the two functions above the test module. `register_image` keeps `#[allow(dead_code)]` because its command-dispatch caller lands in B5; a single allow on `register_image` transitively keeps `parse_qemu_img_info`, `QemuImgInfo`, `write_image_entry`, and `canonical_or_self` reachable, so no other allow attributes are needed in this file (verified against `cargo clippy --all-targets -- -D warnings`). The `let path = ...` chain in `register_image` is written in rustfmt's canonical closure-block form so `cargo fmt --check` stays clean:

```rust
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use toml_edit::{value, DocumentMut, Item, Table};

use super::arch::GuestArch;
use super::discovery::{parse_image_overrides, Firmware, ImageCandidate};
```

```rust
#[allow(dead_code)]
pub(crate) fn register_image(
    emu_toml: &Path,
    candidate: &ImageCandidate,
    qemu_img: &Path,
) -> Result<String> {
    let path = candidate.path.to_str().ok_or_else(|| {
        anyhow!(
            "image path is not valid UTF-8: {}",
            candidate.path.display()
        )
    })?;
    let output = Command::new(qemu_img)
        .args(["info", "--output=json", path])
        .output()
        .with_context(|| format!("failed to run {}", qemu_img.display()))?;
    if !output.status.success() {
        return Err(anyhow!(
            "qemu-img info failed for {}: {}",
            candidate.path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let _info = parse_qemu_img_info(&stdout)?;
    write_image_entry(emu_toml, candidate)
}

fn canonical_or_self(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn write_image_entry(emu_toml: &Path, candidate: &ImageCandidate) -> Result<String> {
    let existing = match std::fs::read_to_string(emu_toml) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", emu_toml.display()))
        }
    };
    let overrides = parse_image_overrides(&existing, None)
        .with_context(|| format!("failed to parse {}", emu_toml.display()))?;
    let registered: HashSet<PathBuf> = overrides
        .values()
        .map(|(path, _, _)| canonical_or_self(path))
        .collect();
    let candidate_canonical = canonical_or_self(&candidate.path);
    if registered.contains(&candidate_canonical) || overrides.contains_key(&candidate.id) {
        return Ok(candidate.id.clone());
    }
    let mut document = existing
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse {}", emu_toml.display()))?;
    let mut table = Table::new();
    table.insert("path", value(candidate.path.to_string_lossy().into_owned()));
    table.insert("arch", value(candidate.arch.as_str()));
    table.insert("firmware", value(candidate.firmware.as_str()));
    let images = document
        .entry("images")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| anyhow!("`images` in {} is not a table", emu_toml.display()))?;
    images.insert(&candidate.id, Item::Table(table));
    std::fs::write(emu_toml, document.to_string())
        .with_context(|| format!("failed to write {}", emu_toml.display()))?;
    Ok(candidate.id.clone())
}
```

Notes:
- `parse_image_overrides` already rejects malformed TOML (its `toml::from_str` returns `Err`), satisfying `fails_on_malformed_toml_without_appending` before any write occurs.
- The id-dedup shares discovery's canonical basis: both the candidate path and the map values are run through `canonical_or_self` (which mirrors `dedupe.rs`'s `canonicalize().unwrap_or_else(self)`), so the comparison basis matches discovery (RR3-CANON-BASIS).
- `value(candidate.arch.as_str())` and `value(candidate.firmware.as_str())` accept `&'static str` directly (toml_edit's `value` is generic over `Into<Value>`, which is implemented for `&str`); `value(candidate.path.to_string_lossy().into_owned())` passes an owned `String`. Both forms verified to compile against toml_edit 0.25.12.

- [ ] Step 4: Run test to verify it passes.

```bash
cargo test -p qol --lib commands::emu::registry
```

Expected: PASS (all registry tests: parse, write-preserve, dup-skip, malformed-fail, canonical-dedup).

- [ ] Step 5: Full gate - build, all emu tests, clippy with `-D warnings`, fmt check.

```bash
cargo test -p qol --lib commands::emu && cargo clippy -p qol --all-targets -- -D warnings && cargo fmt -p qol -- --check
```

Expected: PASS.

- [ ] Step 6: Commit.

```bash
git add tools/qol-cli/src/commands/emu/registry.rs
git commit -m "feat(emu): register_image validates via qemu-img and writes with toml_edit"
```

---

Implementation notes for the executor:
- The `discovery` module must re-export `parse_image_overrides`, `Firmware`, and `ImageCandidate` so `registry.rs` can `use super::discovery::{parse_image_overrides, Firmware, ImageCandidate};`. `parse_image_overrides` is promoted to `pub(crate)` in Commit 3; ensure `discovery/mod.rs` also `pub(crate) use`s it from `config`. `Firmware`/`ImageCandidate` are introduced by B2 - adjust the `use` paths to wherever B2 actually placed them. B2 placed `Firmware` in `arch.rs` (re-exported from `emu.rs`) and `ImageCandidate` in `discovery/candidate.rs` (re-exported from `discovery/mod.rs`); `discovery/mod.rs` must therefore also `pub(crate) use arch::Firmware` (or re-export it) for `super::discovery::Firmware` to resolve. If a path does not resolve, fix the re-export in `discovery/mod.rs` rather than duplicating the type.
- Dead-code discipline (load-bearing, verified against `cargo clippy -p qol --all-targets -- -D warnings`): `--all-targets` compiles the lib crate WITHOUT `cfg(test)`, so any `pub(crate)`/private item whose only caller is a `#[cfg(test)]` test is flagged `function/struct is never used`. That is why Commit 4's inference helpers + `infer_candidate` and Commit 5's `QemuImgInfo`/`parse_qemu_img_info` carry `#[allow(dead_code)]`, and why `host_native_arch` is omitted entirely (it had no caller at all). In Commit 6 a single `#[allow(dead_code)]` on `register_image` transitively keeps the registry chain reachable, so the Commit 5 allows are removed there. When B5 references these symbols from non-test code, remove the remaining `#[allow(dead_code)]` markers.
- Resolver-vs-inference are distinct, non-overlapping concerns. B3's `infer_firmware(arch, name) -> Firmware` answers "what firmware mode should an image default to" from filename hints. B4's `GuestArch::firmware_file(arch, firmware) -> Vec<&'static str>` answers "which firmware blob filenames implement that mode" for the locate step. There is exactly one function per question; B3 does NOT add any arch+firmware->blob-filename function (no `candidate_firmware_filename`), so there is no duplication for B4 to consolidate.

### Task B4: Firmware selection resolve chain

Prereq: B3 has landed. It introduced `enum Firmware { Bios, Uefi }` (derives `Clone, Copy, Debug, PartialEq, Eq`; `as_str` `"bios"`/`"uefi"`) and added the field `firmware: Firmware` to `Environment` (no `Default`; all five literals already set it). B4 is **selection only** - it does not touch parsing or the `Environment` shape. It (1) replaces `GuestArch::firmware_file(self) -> Option<&'static str>` with `firmware_file(arch, firmware) -> Vec<&'static str>`, (2) widens `locate_firmware` to a multi-candidate search keyed on `(arch, firmware)`, (3) repoints the production caller and the test to pass `environment.firmware`, and (4) leaves the existing `pflash` wiring in `qemu_args` unchanged (q35 + OVMF already flows through the same `if let Some(firmware)` arm as aarch64 `virt`).

The wiring in `qemu_args` (emu.rs:853-861) already emits `-drive if=pflash,format=raw,readonly=on,file=<firmware>` for any `Some(firmware)`, regardless of arch. Once `locate_firmware` returns `Ok(Some(path))` for an x86 UEFI environment, q35 + OVMF is wired via pflash with **no change to `qemu_args`**. So B4 makes no edit to `qemu_args`; the "wire via pflash" requirement is satisfied by the existing arch-shared arm.

**Single-source resolver invariant.** B4's `GuestArch::firmware_file(arch, firmware) -> Vec<&'static str>` is the one and only arch+firmware->blob-filename map in the crate. B3 added no competing function (it added `infer_firmware`, a distinct mode-from-filename inference). B4 therefore introduces the resolver fresh, not as a consolidation of two.

**B4 updates every caller of the old arity in the same commit.** Replacing `GuestArch::firmware_file(self) -> Option<&'static str>` changes the arity to `(arch, firmware) -> Vec<..>`. The sole remaining caller is `locate_firmware` (emu.rs:761-777, which destructures the old `Option`); B4a rewrites it in the same atomic commit so no caller of the old single-arg form survives. B3 must not add any new call to the old arity in the interim - B3 touches firmware via `Environment.firmware` and `infer_firmware` only, never `GuestArch::firmware_file`.

**Why B4a is one commit.** Replacing `firmware_file(self)` and rewriting `locate_firmware` (the sole remaining caller of `firmware_file`, at emu.rs:765) and repointing the production caller (emu.rs:723) are mutually dependent: changing `firmware_file`'s arity alone leaves emu.rs:765 calling the old zero-arg form, so the crate does not compile. The repo rule is that each commit compiles and represents a working state, so all three edits plus both tests land in **one atomic commit** (Task B4a). Task B4b is the verification-only lint gate.

---

#### Task B4a: `firmware_file(arch, firmware)` + multi-candidate `locate_firmware`, caller and tests

This single task: replaces `firmware_file` with the two-argument `(arch, firmware) -> Vec<&'static str>` form, rewrites `locate_firmware` to search the candidate list across the QEMU-relative dir plus fixed fallbacks, repoints the production caller at emu.rs:723 to pass `environment.firmware`, and widens both the `arch.rs` and `emu.rs` tests. `(X86_64, Bios)` stays `None`-equivalent (empty vec -> `Ok(None)`); `(X86_64, Uefi)` yields the three OVMF/edk2 names; `(Aarch64, _)` yields the single arm edk2 name. An empty candidate list short-circuits to `Ok(None)` (the no-blob case - x86 BIOS, no regression). Otherwise each candidate filename is searched across `<bin>/../share/qemu` plus `/usr/share/qemu`, `/usr/share/OVMF`, `/usr/share/edk2/x64`, first hit wins; if none resolve in any dir it returns `Err(reason)` naming the candidates, which `resolve_environment` already maps to `Unsupported` (emu.rs:725-735).

**Files:**
- Modify: `tools/qol-cli/src/commands/emu/arch.rs:39-44` (replace `firmware_file`)
- Modify: `tools/qol-cli/src/commands/emu.rs:761-777` (replace `locate_firmware`, add `FIRMWARE_FALLBACK_DIRS`)
- Modify: `tools/qol-cli/src/commands/emu.rs:723` (production caller passes `environment.firmware`)
- Test: `tools/qol-cli/src/commands/emu/arch.rs:47-74` (add a case into the `tests` module)
- Test: `tools/qol-cli/src/commands/emu.rs:1145-1165` (rewrite `locate_firmware_finds_edk2_next_to_binary`)

- [ ] Step 1: Write the failing tests.

First, add this test inside the existing `#[cfg(test)] mod tests` block in `tools/qol-cli/src/commands/emu/arch.rs`, after `qemu_system_binary_is_arch_suffixed` (before the closing `}` of the module). `Firmware` is already in scope in `arch.rs` (B2 defined it there), so no extra `use` is needed.

```rust
    #[test]
    fn firmware_file_selection_by_arch_and_mode() {
        let cases: [(GuestArch, Firmware, Vec<&str>); 4] = [
            (GuestArch::X86_64, Firmware::Bios, vec![]),
            (
                GuestArch::X86_64,
                Firmware::Uefi,
                vec![
                    "edk2-x86_64-code.fd",
                    "OVMF_CODE.fd",
                    "OVMF_CODE_4M.fd",
                ],
            ),
            (
                GuestArch::Aarch64,
                Firmware::Bios,
                vec!["edk2-aarch64-code.fd"],
            ),
            (
                GuestArch::Aarch64,
                Firmware::Uefi,
                vec!["edk2-aarch64-code.fd"],
            ),
        ];
        for (arch, firmware, expected) in cases {
            assert_eq!(
                GuestArch::firmware_file(arch, firmware),
                expected,
                "arch: {arch:?}, firmware: {firmware:?}"
            );
        }
    }
```

Then replace the entire existing `locate_firmware_finds_edk2_next_to_binary` test (emu.rs:1145-1165) with the widened version below. `Firmware` is in scope here via the test module's `use super::*;` (it is a `pub(crate)` re-export sibling of `locate_firmware` in `emu.rs`).

```rust
    #[test]
    fn locate_firmware_finds_edk2_next_to_binary() {
        let root = std::env::temp_dir().join(format!("qol-emu-fw-{}", std::process::id()));
        let bin = root.join("bin");
        let share = root.join("share/qemu");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&share).unwrap();
        let qemu_system = bin.join("qemu-system-aarch64");
        fs::write(&qemu_system, b"x").unwrap();

        assert_eq!(
            locate_firmware(&qemu_system, GuestArch::X86_64, Firmware::Bios),
            Ok(None)
        );

        let arm_missing =
            locate_firmware(&qemu_system, GuestArch::Aarch64, Firmware::Uefi).unwrap_err();
        assert!(
            arm_missing.contains("edk2-aarch64-code.fd"),
            "error: {arm_missing}"
        );
        let x86_missing =
            locate_firmware(&qemu_system, GuestArch::X86_64, Firmware::Uefi).unwrap_err();
        assert!(
            x86_missing.contains("OVMF_CODE.fd"),
            "error: {x86_missing}"
        );

        fs::write(share.join("edk2-aarch64-code.fd"), b"fw").unwrap();
        let arm_found = locate_firmware(&qemu_system, GuestArch::Aarch64, Firmware::Uefi)
            .unwrap()
            .unwrap();
        assert!(
            arm_found.ends_with("edk2-aarch64-code.fd"),
            "found: {arm_found:?}"
        );

        fs::write(share.join("OVMF_CODE.fd"), b"fw").unwrap();
        let x86_found = locate_firmware(&qemu_system, GuestArch::X86_64, Firmware::Uefi)
            .unwrap()
            .unwrap();
        assert!(x86_found.ends_with("OVMF_CODE.fd"), "found: {x86_found:?}");

        fs::remove_dir_all(&root).unwrap();
    }
```

- [ ] Step 2: Run the tests to verify they fail.

```bash
cargo test -p qol --lib commands::emu 2>&1 | tail -30
```

Expected: FAIL to compile - `firmware_file` still takes only `self` and returns `Option<&'static str>`, and `locate_firmware` still takes `(qemu_system, arch)`, so the three-argument call sites and `GuestArch::firmware_file(arch, firmware)` do not type-check.

- [ ] Step 3: Write the minimal implementation. Three edits.

Edit 1 - in `tools/qol-cli/src/commands/emu/arch.rs`, replace the existing `firmware_file` method (lines 39-44) with the two-argument form. `Firmware` is already in scope in `arch.rs` (B2 defined it there), so no file-scope import is added:

```rust
    pub(crate) fn firmware_file(self, firmware: Firmware) -> Vec<&'static str> {
        match (self, firmware) {
            (GuestArch::X86_64, Firmware::Bios) => vec![],
            (GuestArch::X86_64, Firmware::Uefi) => vec![
                "edk2-x86_64-code.fd",
                "OVMF_CODE.fd",
                "OVMF_CODE_4M.fd",
            ],
            (GuestArch::Aarch64, _) => vec!["edk2-aarch64-code.fd"],
        }
    }
```

Edit 2 - in `tools/qol-cli/src/commands/emu.rs`, replace the whole `locate_firmware` function (lines 761-777) with the multi-candidate version, introducing a module-level `const FIRMWARE_FALLBACK_DIRS` immediately above it. `Firmware` is already in scope in this file (B2 re-exported it; B3 added the `firmware: Firmware` field to `Environment`). The nested `if let Ok(path) = ... { if path.is_file() { ... } }` is intentional and edition-2021-correct (let-chains are not available on edition 2021); it is clippy-clean per the codebase nesting rule.

```rust
const FIRMWARE_FALLBACK_DIRS: [&str; 3] =
    ["/usr/share/qemu", "/usr/share/OVMF", "/usr/share/edk2/x64"];

fn locate_firmware(
    qemu_system: &Path,
    arch: GuestArch,
    firmware: Firmware,
) -> std::result::Result<Option<PathBuf>, String> {
    let candidates = arch.firmware_file(firmware);
    if candidates.is_empty() {
        return Ok(None);
    }
    let Some(bin_dir) = qemu_system.parent() else {
        return Err(format!("{} has no parent directory", qemu_system.display()));
    };
    let mut search_dirs = vec![bin_dir.join("../share/qemu")];
    search_dirs.extend(FIRMWARE_FALLBACK_DIRS.iter().map(PathBuf::from));
    for dir in &search_dirs {
        for file in &candidates {
            let candidate = dir.join(file);
            if let Ok(path) = candidate.canonicalize() {
                if path.is_file() {
                    return Ok(Some(path));
                }
            }
        }
    }
    Err(format!(
        "missing firmware ({}) under {}",
        candidates.join(", "),
        search_dirs
            .iter()
            .map(|dir| dir.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}
```

Edit 3 - repoint the production caller at emu.rs:723. Change:

```rust
        Some(path) => match locate_firmware(path, environment.arch) {
```

to:

```rust
        Some(path) => match locate_firmware(path, environment.arch, environment.firmware) {
```

- [ ] Step 4: Build the whole crate and run the affected tests.

```bash
cargo build -p qol 2>&1 | tail -15 && cargo test -p qol --lib commands::emu 2>&1 | tail -30
```

Expected: PASS - `firmware_file_selection_by_arch_and_mode`, `locate_firmware_finds_edk2_next_to_binary`, and the existing `qemu_args_wire_aarch64_machine_cpu_and_firmware` / `qemu_args_wire_accel_display_and_qmp` all green; the build is clean with no stale single-arg caller.

- [ ] Step 5: Commit.

```bash
git add tools/qol-cli/src/commands/emu/arch.rs tools/qol-cli/src/commands/emu.rs
git commit -m "feat(emu): select and locate firmware blobs by arch and mode"
```

---

#### Task B4b: Lint gate

Confirm no `-D warnings` regressions across the crate (CI gate). The new `Vec` return, the `FIRMWARE_FALLBACK_DIRS` const, and the widened signatures must be warning-free; the previously single-use `arch` branch in `locate_firmware` is now exercised by both arches.

**Files:** none (verification only).

- [ ] Step 1: Run clippy with warnings denied.

```bash
cargo clippy -p qol --all-targets -- -D warnings 2>&1 | tail -25
```

Expected: PASS - no warnings. `candidates.join(", ")` on `Vec<&str>` is fine; the nested `if let Ok(path) = ... { if path.is_file() {` is intentional and acceptable per the codebase nesting rule. The crate is edition 2021, so do **not** rewrite this into a let-chain (`if let Ok(path) = ... && path.is_file()`) - let-chains require edition 2024 and are a hard compile error here; clippy will not suggest the collapse on edition 2021. If clippy reports any genuine warning, fix it and re-run before proceeding.

- [ ] Step 2: This is a verification-only task; if Step 1 was clean, no commit. If a genuine clippy fix was required, commit it.

```bash
git add tools/qol-cli/src/commands/emu.rs tools/qol-cli/src/commands/emu/arch.rs
git commit -m "refactor(emu): satisfy clippy on firmware locate chain"
```

---

**End-of-step DoD (spec lines 549-554):** x86-BIOS still resolves `Ok(None)` (no gate, no regression); an x86-UEFI environment with OVMF present resolves `Ready` (firmware located, wired via the existing pflash arm in `qemu_args`), and with OVMF absent resolves `Unsupported` with a reason naming the candidates. The full crate builds and `cargo clippy -p qol --all-targets -- -D warnings` is clean.

### Task B5: `qol emu add`/`open` CLI + `PlatformOps::open_path` + TUI `o`/`t`/`a`

**Prerequisite symbols from earlier steps (already in the tree when B5 runs).** A1-A3, B1-B4, B2e, and B5a/B5b have landed. This step references:
- `Firmware` enum (`Bios`/`Uefi`) with `Firmware::as_str(self) -> &'static str` and `Firmware::parse(&str) -> Option<Firmware>` (B2/B3), in `arch.rs`, re-exported from `emu.rs`.
- `ImageCandidate { id, path, display_name, arch, arch_inferred, firmware }` (derives `Clone, Debug, PartialEq, Eq`; all fields `pub(crate)`), in `commands/emu/discovery/candidate.rs`, re-exported `pub(crate) use` from `discovery/mod.rs` AND from `emu.rs` (B2/B2e), so `commands::emu::ImageCandidate` resolves crate-wide.
- `Discovered { environments, candidates }`, re-exported the same way (B2/B2e).
- `emu_scan() -> Result<(Vec<EnvironmentStatus>, Vec<ImageCandidate>)>` in `commands/emu.rs` (B2e), one discovery pass producing both statuses and candidates.
- `register_image(emu_toml: &Path, candidate: &ImageCandidate, qemu_img: &Path) -> Result<String>`, re-exported as `commands::emu::register_image` (B3/B5b).
- `emu_dir() -> Option<PathBuf>` and `emu_config_path() -> Option<PathBuf>`, both `pub(crate)` (B1/A3); `find_on_path` (emu.rs:799, `pub(crate)`); `GuestArch`, re-exported as `commands::emu::GuestArch`.
- `host_facade::open_path(&Path)` (B5a).

The candidate model: candidates live in a NEW sibling `Dash` field `emu_candidates: Vec<ImageCandidate>` added in B5c, NOT inside `EmuState::Done`. The emu poller payload becomes `Result<(Vec<EnvironmentStatus>, Vec<ImageCandidate>), String>`; the poll-wiring Ok arm stores candidates into `dash.emu_candidates` and sets `EmuState::Done(statuses)` unchanged. Therefore `emu_status` and the four existing `EmuState::Done(vec![...])` tests are NOT touched. The cursor runs over environments first, then candidates: down-clamp is `(emu_env_count(dash) + dash.emu_candidates.len()).saturating_sub(1)`, and `selected_candidate_mut` reads `dash.emu_cursor.checked_sub(emu_env_count(dash)).and_then(|i| dash.emu_candidates.get_mut(i))`: plain field access, one form, no thread-local, no `matches!` guard.

---

#### Task B5a: `PlatformOps::open_path` (per-OS argv, unit-tested without spawn)

Add a sibling to `open_url` that opens a directory in the OS file manager. To make it unit-testable with "no spawn", each per-OS impl delegates to a pure `open_path_argv(dir) -> (&'static str, Vec<String>)` free function that the trait method then spawns. The trait method itself is `fn open_path(&self, dir: &Path)` (void, mirroring `open_url`).

**Files:**
- Modify: `tools/qol-cli/src/platform/mod.rs` (trait at 23-29)
- Modify: `tools/qol-cli/src/platform/macos.rs` (impl at 7-33; `open_url` at 21-28)
- Modify: `tools/qol-cli/src/platform/linux.rs` (impl at 7-36; `open_url` at 21-28)
- Modify: `tools/qol-cli/src/platform/windows.rs` (impl at 7-30; `open_url` at 23-25)
- Modify: `tools/qol-cli/src/platform/unsupported.rs` (impl at 6-24; `open_url` at 19)
- Modify: `tools/qol-cli/src/host_facade.rs` (wrapper functions)
- Test: `tools/qol-cli/src/platform/macos.rs` (new `#[cfg(test)] mod tests`, only compiled on macOS)

- [ ] Step 1: Write the failing test. Append a test module to the END of `tools/qol-cli/src/platform/macos.rs` asserting the pure argv builder. This test is only compiled on the macOS target (the whole file is `#[cfg(target_os = "macos")]`-gated via `mod.rs:7-8`), so it exercises the macOS branch without spawning.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn open_path_argv_uses_open_with_dir_argument() {
        let (program, args) = open_path_argv(Path::new("/a/b/emu"));
        assert_eq!(program, "open", "program");
        assert_eq!(args, vec!["/a/b/emu".to_string()], "args");
    }
}
```

- [ ] Step 2: Run test to verify it fails.

```bash
cargo test -p qol --lib platform::macos::tests::open_path_argv_uses_open_with_dir_argument
```

Expected: FAIL to compile - `open_path_argv` is not defined.

- [ ] Step 3: Write minimal implementation.

First add the trait method in `tools/qol-cli/src/platform/mod.rs`, replacing the trait body:

```rust
pub(crate) trait PlatformOps {
    fn os_name(&self) -> &'static str;
    fn exe_name(&self, name: &str) -> String;
    fn stop_qol_tray(&self) -> Result<()>;
    fn open_url(&self, url: &str);
    fn open_path(&self, dir: &Path);
    fn copy_to_clipboard(&self, text: &str) -> Result<()>;
}
```

Add the `Path` import to the top of `tools/qol-cli/src/platform/mod.rs` (alongside the existing `use std::io::Write;` / `use std::process::{Command, Stdio};` lines):

```rust
use std::path::Path;
```

In `tools/qol-cli/src/platform/macos.rs`, change the import line `use std::process::{Command, Stdio};` to two lines so `Path` is in scope:

```rust
use std::path::Path;
use std::process::{Command, Stdio};
```

Insert the `open_path` impl method directly after the `open_url` method block (lines 21-28):

```rust
    fn open_path(&self, dir: &Path) {
        let (program, args) = open_path_argv(dir);
        let _ = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
```

And add the free function above the test module (outside the `impl` block):

```rust
fn open_path_argv(dir: &Path) -> (&'static str, Vec<String>) {
    ("open", vec![dir.display().to_string()])
}
```

In `tools/qol-cli/src/platform/linux.rs`, add `use std::path::Path;` to the imports and insert after the `open_url` method:

```rust
    fn open_path(&self, dir: &Path) {
        let (program, args) = open_path_argv(dir);
        let _ = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
```

Add the linux argv builder after the `impl` block:

```rust
fn open_path_argv(dir: &Path) -> (&'static str, Vec<String>) {
    ("xdg-open", vec![dir.display().to_string()])
}
```

In `tools/qol-cli/src/platform/windows.rs`, add `use std::path::Path;` to the imports and insert after the `open_url` method:

```rust
    fn open_path(&self, dir: &Path) {
        let (program, args) = open_path_argv(dir);
        let _ = Command::new(program).args(args).spawn();
    }
```

Add the windows argv builder after the `impl` block:

```rust
fn open_path_argv(dir: &Path) -> (&'static str, Vec<String>) {
    ("explorer", vec![dir.display().to_string()])
}
```

In `tools/qol-cli/src/platform/unsupported.rs`, add `use std::path::Path;` to the imports and insert after the `open_url` no-op:

```rust
    fn open_path(&self, _dir: &Path) {}
```

Finally add a `host_facade` wrapper in `tools/qol-cli/src/host_facade.rs`, after `open_url`:

```rust
pub(crate) fn open_path(dir: &std::path::Path) {
    Platform.open_path(dir)
}
```

- [ ] Step 4: Run test to verify it passes.

```bash
cargo test -p qol --lib platform::macos::tests::open_path_argv_uses_open_with_dir_argument
cargo clippy -p qol --all-targets -- -D warnings
```

Expected: PASS. (`host_facade::open_path` is unused until B5b/B5c add callers; on CI the cross-platform `-D warnings` gate will not flag it because B5b wires the CLI caller in this same Task sequence. If running B5a in isolation triggers a `dead_code` warning on `host_facade::open_path`, proceed directly to B5b which adds the caller - do not gate it behind `#[allow]`.)

- [ ] Step 5: Commit.

```bash
git add tools/qol-cli/src/platform/mod.rs tools/qol-cli/src/platform/macos.rs tools/qol-cli/src/platform/linux.rs tools/qol-cli/src/platform/windows.rs tools/qol-cli/src/platform/unsupported.rs tools/qol-cli/src/host_facade.rs
git commit -m "feat(emu): add PlatformOps::open_path per-OS opener"
```

---

#### Task B5b: `qol emu add` and `qol emu open` CLI verbs

Add two verbs to the `run()` dispatch. `add` builds an `ImageCandidate` from `<path>` via `infer_candidate`, applies `--arch`/`--firmware`/`--id` overrides (`--id` sanitized through the same path as a filename-derived id, never rejected), then calls the single `register_image` contract. `open` resolves `emu_dir()`, creates it if missing, and opens it (or prints the path on a headless host).

The id-override path reuses `sanitize_id` (pre-existing, `commands/emu.rs:1011`). The collision suffix is already applied by `register_image`'s dedup step (B3); the CLI only sanitizes the raw `--id` string and sets it on the candidate before handing off.

**Files:**
- Modify: `tools/qol-cli/src/commands/emu.rs` (`run` dispatch at 87-106; `emu_help_text` at 682-684; new `cmd_add`/`cmd_open` + arg-parse helper; remove `infer_candidate`/`register_image` `#[allow(dead_code)]`; `#[cfg(test)] mod tests` at 1047)
- Test: `tools/qol-cli/src/commands/emu.rs` (`#[cfg(test)] mod tests`)

- [ ] Step 1: Write the failing test. Add to the existing `#[cfg(test)] mod tests` in `tools/qol-cli/src/commands/emu.rs` a test for the pure arg-parser `parse_add_args`, which returns the overrides without touching the filesystem. (`super::*` already brings `OsString`, `PathBuf`, `GuestArch`, and `Firmware` into the test module's scope.)

```rust
    #[test]
    fn parse_add_args_extracts_path_and_overrides() {
        let args: Vec<OsString> = ["/a/b/win.qcow2", "--arch", "aarch64", "--firmware", "uefi", "--id", "My Box!"]
            .iter()
            .map(OsString::from)
            .collect();
        let parsed = parse_add_args(&args).unwrap();
        assert_eq!(parsed.path, PathBuf::from("/a/b/win.qcow2"), "path");
        assert_eq!(parsed.arch, Some(GuestArch::Aarch64), "arch");
        assert_eq!(parsed.firmware, Some(Firmware::Uefi), "firmware");
        assert_eq!(parsed.id.as_deref(), Some("my-box"), "id sanitized");
    }

    #[test]
    fn parse_add_args_requires_a_path() {
        let args: Vec<OsString> = ["--arch", "x86_64"].iter().map(OsString::from).collect();
        assert!(parse_add_args(&args).is_err(), "missing path must error");
    }

    #[test]
    fn parse_add_args_rejects_unknown_arch_and_firmware() {
        let bad_arch: Vec<OsString> = ["/a/b/x.img", "--arch", "riscv"].iter().map(OsString::from).collect();
        assert!(parse_add_args(&bad_arch).is_err(), "unknown arch must error");
        let bad_fw: Vec<OsString> = ["/a/b/x.img", "--firmware", "coreboot"].iter().map(OsString::from).collect();
        assert!(parse_add_args(&bad_fw).is_err(), "unknown firmware must error");
    }
```

- [ ] Step 2: Run test to verify it fails.

```bash
cargo test -p qol --lib commands::emu::tests::parse_add_args
```

Expected: FAIL to compile - `parse_add_args` and `AddArgs` are not defined.

- [ ] Step 3: Write minimal implementation.

`Firmware` is already in scope in `commands/emu.rs` (B2's `pub(crate) use arch::{Firmware, GuestArch};`). `infer_candidate` is referenced as `discovery::infer_candidate`; `register_image` is re-exported via `pub(crate) use registry::register_image;` (add this re-export next to the `mod registry;` declaration if not already present from B3); `emu_dir`/`emu_config_path`/`sanitize_id`/`find_on_path`/`GuestArch` are already crate-visible. Remove the `#[allow(dead_code)]` markers from `infer_candidate` (filesystem.rs) and `register_image` (registry.rs) now that B5 calls them from non-test code.

Add the parser struct and function near the other `fn cmd_*` helpers (e.g. above `print_emu_help` at line 678):

```rust
struct AddArgs {
    path: PathBuf,
    arch: Option<GuestArch>,
    firmware: Option<Firmware>,
    id: Option<String>,
}

fn parse_add_args(args: &[OsString]) -> Result<AddArgs> {
    let mut path: Option<PathBuf> = None;
    let mut arch: Option<GuestArch> = None;
    let mut firmware: Option<Firmware> = None;
    let mut id: Option<String> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.to_str() {
            Some("--arch") => {
                let value = iter
                    .next()
                    .and_then(|value| value.to_str())
                    .context("--arch needs a value")?;
                arch = Some(
                    GuestArch::parse(value)
                        .ok_or_else(|| anyhow!("--arch must be one of: x86_64, aarch64"))?,
                );
            }
            Some("--firmware") => {
                let value = iter
                    .next()
                    .and_then(|value| value.to_str())
                    .context("--firmware needs a value")?;
                firmware = Some(
                    Firmware::parse(value)
                        .ok_or_else(|| anyhow!("--firmware must be one of: bios, uefi"))?,
                );
            }
            Some("--id") => {
                let value = iter
                    .next()
                    .and_then(|value| value.to_str())
                    .context("--id needs a value")?;
                id = Some(sanitize_id(value));
            }
            _ => {
                if path.is_some() {
                    bail!("usage: qol emu add <path> [--arch x86_64|aarch64] [--firmware bios|uefi] [--id <id>]");
                }
                path = Some(PathBuf::from(arg));
            }
        }
    }
    Ok(AddArgs {
        path: path.context(
            "usage: qol emu add <path> [--arch x86_64|aarch64] [--firmware bios|uefi] [--id <id>]",
        )?,
        arch,
        firmware,
        id,
    })
}
```

Add the two command functions below `parse_add_args`:

```rust
fn cmd_add(args: &[OsString], verbose: bool) -> Result<()> {
    print_title("qol emu add");
    print_hint(verbose);
    let parsed = parse_add_args(args)?;
    let mut candidate = discovery::infer_candidate(&parsed.path);
    if let Some(arch) = parsed.arch {
        candidate.arch = arch;
        candidate.arch_inferred = true;
    }
    if let Some(firmware) = parsed.firmware {
        candidate.firmware = firmware;
    }
    if let Some(id) = parsed.id {
        candidate.id = id;
    }
    let qemu_img = find_on_path("qemu-img").context("missing qemu-img")?;
    let emu_toml = emu_config_path().context("could not determine emu.toml path")?;
    let id = register_image(&emu_toml, &candidate, &qemu_img)?;
    step_label("add", StepKind::Info, &format!("registered {id}"));
    Ok(())
}

fn cmd_open(args: &[OsString], _verbose: bool) -> Result<()> {
    if !args.is_empty() {
        bail!("usage: qol emu open");
    }
    let dir = emu_dir().context("could not determine emu dir")?;
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    print_title("qol emu open");
    step_label("dir", StepKind::Info, &dir.display().to_string());
    if env::var_os("DISPLAY").is_none()
        && env::var_os("WAYLAND_DISPLAY").is_none()
        && crate::host_facade::os_name() == "linux"
    {
        return Ok(());
    }
    crate::host_facade::open_path(&dir);
    Ok(())
}
```

> NOTE on `emu_dir()` return type: B1 defined `emu_dir() -> Option<PathBuf>`. `cmd_open` above handles `None` with `.context("could not determine emu dir")?`. If the executor prefers `emu_dir() -> PathBuf` (infallible, falling back to a default), reconcile B1's signature first; do not call `emu_dir()` as if it were infallible. The `print_hint`/`print_title`/`print_emu_help` helper names must match the real `emu.rs` helpers (verify against the current file; if the title/hint helpers differ, substitute the actual ones).

Add the dispatch arms in `run()` (between the existing arms inside `match command`, e.g. after `"list" => cmd_list(rest, verbose),`):

```rust
        "add" => cmd_add(rest, verbose),
        "open" => cmd_open(rest, verbose),
```

Update `emu_help_text` to list the new verbs by replacing the returned string so it begins with `add` and `open` after `list`:

```rust
    "qol emu commands:\n  qol emu list\n  qol emu add <path> [--arch x86_64|aarch64] [--firmware bios|uefi] [--id <id>]\n  qol emu open\n  qol emu doctor\n  qol emu up <environment>\n  qol emu run <workflow> <environment>\n  qol emu check <environment>\n  qol emu shot <environment>\n  qol emu key <environment> <qcode>...\n  qol emu insert <environment>\n  qol emu pull <environment>\n  qol emu snap <environment>\n  qol emu sh <environment> <command>...\n  qol emu down <environment>\n\nControl verbs target the newest running `qol emu up` for that environment.\n\nEmus are discovered from libvirt/QEMU domains plus optional local config:\n  ~/.config/qol-tray/emu.toml\n\nExample config:\n  [images]\n  my-windows = \"/path/to/windows.qcow2\"\n"
```

- [ ] Step 4: Run test to verify it passes.

```bash
cargo test -p qol --lib commands::emu::tests::parse_add_args
cargo build -p qol
cargo clippy -p qol --all-targets -- -D warnings
```

Expected: PASS.

- [ ] Step 5: Commit.

```bash
git add tools/qol-cli/src/commands/emu.rs tools/qol-cli/src/commands/emu/discovery/filesystem.rs tools/qol-cli/src/commands/emu/registry.rs
git commit -m "feat(emu): add qol emu add and qol emu open CLI verbs"
```

---

#### Task B5c: Surface candidates into the TUI (poller + Dash.emu_candidates + draw_emu + candidate_row_label)

Widen the emu poller payload to `(Vec<EnvironmentStatus>, Vec<ImageCandidate>)` by calling `emu_scan`, add a sibling `Dash.emu_candidates` field, store candidates from the poll-wiring Ok arm, extend the down-cursor clamp over env+candidate count, define the pure `candidate_row_label` helper, and render candidate rows after the environment rows in `draw_emu`. `EmuState::Done` keeps carrying only `Vec<EnvironmentStatus>`.

`candidate_row_label` is defined HERE (not in B5d) so that B5c compiles and clippies standalone: B5c's `draw_emu` is its first caller. B5d adds only the key handlers and side-effecting helpers.

This is a poller-payload + struct-field + render rewire with one pure-function seam (`candidate_row_label`). Verify with `cargo build -p qol` + `cargo clippy -p qol --all-targets -- -D warnings` plus the existing `cargo test -p qol --bin qol` runs and one new label test.

Files:
- Modify: `tools/qol-cli/src/dev_console.rs` (`Probes.emu` field type; `Probes::spawn` emu closure; poll-wiring Ok arm; `struct Dash` fields; `Dash::new`; `ScrollDown` View::Emu clamp; `draw_emu`; `candidate_row_label` near `draw_emu`)

---

**Step 1: Add the `emu_candidates` field to `Dash` and fix the import.** Add the field to `struct Dash`, after `emu_cursor: usize,`:

```rust
    emu_candidates: Vec<ImageCandidate>,
```

`ImageCandidate` must be in scope and `environment_statuses` must be dropped (Step 3 makes it unused). B1 set the import to:

```rust
use crate::commands::emu::{
    emu_config_path, emu_dir, environment_statuses, legacy_advisory_count, newest_run_detail,
    EnvironmentStatus, LastRun, ResolveState, RunDetail,
};
```

Replace it with (remove `environment_statuses`, add `emu_scan` and `ImageCandidate`):

```rust
use crate::commands::emu::{
    emu_config_path, emu_dir, emu_scan, legacy_advisory_count, newest_run_detail, ImageCandidate,
    EnvironmentStatus, LastRun, ResolveState, RunDetail,
};
```

`ImageCandidate` resolves through the `pub(crate) use` re-export B2e added to `emu.rs`.

**Step 2: Initialise the field in `Dash::new`.** Add to the `Self { ... }` literal, after `emu_cursor: 0,`:

```rust
            emu_candidates: Vec::new(),
```

**Step 3: Widen the emu poller payload.** Change the `Probes.emu` field type.

**Before**:

```rust
    emu: Poller<Result<Vec<EnvironmentStatus>, String>>,
```

**After**:

```rust
    emu: Poller<Result<(Vec<EnvironmentStatus>, Vec<ImageCandidate>), String>>,
```

Change the emu closure in `Probes::spawn` to call `emu_scan`.

**Before**:

```rust
            emu: Poller::spawn(EMU_REFRESH_INTERVAL, || {
                environment_statuses().map_err(|error| format!("{error:#}"))
            }),
```

**After**:

```rust
            emu: Poller::spawn(EMU_REFRESH_INTERVAL, || {
                emu_scan().map_err(|error| format!("{error:#}"))
            }),
```

This is the only call site of `environment_statuses` in `dev_console.rs`, which is why Step 1 drops it from the import.

**Step 4: Store candidates in the poll-wiring Ok arm.**

**Before**:

```rust
        if let Some(outcome) = probes.emu.latest() {
            dash.emu = match outcome {
                Ok(statuses) => EmuState::Done(statuses),
                Err(error) => EmuState::Failed(error),
            };
        }
```

**After**: the Ok arm splits the tuple, storing candidates into the sibling field and setting `EmuState::Done(statuses)` unchanged:

```rust
        if let Some(outcome) = probes.emu.latest() {
            dash.emu = match outcome {
                Ok((statuses, candidates)) => {
                    dash.emu_candidates = candidates;
                    EmuState::Done(statuses)
                }
                Err(error) => EmuState::Failed(error),
            };
        }
```

**Step 5: Extend the down-cursor clamp over env+candidate count.** Change the `View::Emu` arm of `Action::ScrollDown`.

**Before**:

```rust
            View::Emu => {
                dash.emu_cursor = (dash.emu_cursor + 1).min(emu_env_count(dash).saturating_sub(1))
            }
```

**After**:

```rust
            View::Emu => {
                let total = emu_env_count(dash) + dash.emu_candidates.len();
                dash.emu_cursor = (dash.emu_cursor + 1).min(total.saturating_sub(1));
            }
```

`emu_env_count` and `selected_emu_status` stay unchanged; they read only `EmuState::Done(statuses)`. With `Dash::new` initialising `emu_candidates` to empty, the four `EmuState::Done(vec![...])` tests see the same clamp as before.

**Step 6: Define `candidate_row_label` and render candidate rows in `draw_emu`.** Add the pure label builder above `draw_emu`:

```rust
fn candidate_row_label(arch: crate::commands::emu::GuestArch, arch_inferred: bool) -> String {
    if arch_inferred {
        format!("needs arch · {}", arch.as_str())
    } else {
        format!("needs arch · {} (host default)", arch.as_str())
    }
}
```

`draw_emu` builds `lines` from `match &dash.emu`; that match borrows `dash.emu` immutably and the borrow ends when the owned `Vec<Line<'static>>` is produced. The candidate rows append after that block because they read `dash.emu_candidates` and `emu_env_count(dash)`, then `list_window(dash, ...)` takes its own `&mut`. After the `let lines = match &dash.emu { ... };` block (immediately before `let total = lines.len();`), insert:

```rust
    let mut lines = lines;
    let env_count = emu_env_count(dash);
    for (index, candidate) in dash.emu_candidates.iter().enumerate() {
        let selected = env_count + index == dash.emu_cursor;
        let caret: Span<'static> = if selected {
            "▸ ".fg(Color::Green).bold()
        } else {
            "  ".into()
        };
        let id_span = if selected {
            candidate.id.clone().fg(Color::White).bold()
        } else {
            candidate.id.clone().fg(Color::White)
        };
        lines.push(Line::from(vec![
            caret,
            "○ ".fg(Color::DarkGray),
            id_span,
            format!("  {}", candidate_row_label(candidate.arch, candidate.arch_inferred))
                .fg(Color::DarkGray),
        ]));
    }
```

The cursor math uses `env_count` (the environment COUNT), not `lines.len()`, because a non-Ready environment renders two lines (header + reason) while it still occupies one cursor slot. The candidate loop is purely additive and does not depend on the env line count.

The empty-state branch (`EmuState::Done(statuses) if statuses.is_empty()`) is unchanged. When there are zero envs but some candidates, `draw_emu` falls into that empty arm for the env lines, and the candidate loop appends candidate rows below them, which is correct.

**Step 7: Build, clippy, and confirm the untouched tests pass.**

```bash
cargo build -p qol 2>&1 | tail -20
cargo test -p qol --bin qol dev_console 2>&1 | tail -30
cargo clippy -p qol --all-targets -- -D warnings 2>&1 | tail -30
```

Expected: PASS. The four `EmuState::Done(vec![...])` tests stay green because `EmuState::Done` is unchanged and `Dash::new` initialises `emu_candidates` to empty (cursor clamp with zero candidates equals the old `emu_env_count`-only clamp). `emu_status` unchanged. Clippy clean: `emu_candidates` is read by Steps 5-6, `candidate_row_label` is called by `draw_emu`, and `environment_statuses` is no longer imported.

**Step 8: Commit.**

```bash
git add tools/qol-cli/src/dev_console.rs
git commit -m "feat(emu): surface discovery candidates into the dev console"
```

---

#### Task B5d: TUI `o`/`t`/`a` key handlers

Add three `Action` variants (`OpenEmuDir`, `ToggleArch`, `Confirm`), map the free keys `o`/`t`/`a` in `action_for`, and handle them in `apply_action` gated under `dash.view == View::Emu`. `ToggleArch` flips the selected candidate's arch and sets `arch_inferred = true`. `Confirm` calls `register_image` then pokes the emu poller. `OpenEmuDir` creates `emu_dir` if missing and opens it. `selected_candidate_mut` is the single plain-field form. `candidate_row_label` already exists (B5c).

Files:
- Modify: `tools/qol-cli/src/dev_console.rs` (`Action` enum; `action_for`; `apply_action`; helpers near `draw_emu`; `selected_candidate_mut` near `selected_emu_status`)
- Test: `tools/qol-cli/src/dev_console.rs` (`#[cfg(test)] mod tests`)

---

**Step 1: Write the failing test.** Add to the dev_console test module (`use super::*;`) tests for the label builder and the key map. `GuestArch` is not in the dev_console module scope, so bring it in explicitly inside the test:

```rust
    #[test]
    fn candidate_row_label_marks_host_default() {
        use crate::commands::emu::GuestArch;
        let cases = [
            (GuestArch::Aarch64, true, "needs arch · aarch64"),
            (GuestArch::X86_64, false, "needs arch · x86_64 (host default)"),
        ];
        for (arch, inferred, expected) in cases {
            assert_eq!(
                candidate_row_label(arch, inferred),
                expected,
                "arch: {arch:?} inferred: {inferred}"
            );
        }
    }

    #[test]
    fn emu_keys_map_open_toggle_and_confirm() {
        let cases = [
            (KeyCode::Char('o'), Action::OpenEmuDir),
            (KeyCode::Char('t'), Action::ToggleArch),
            (KeyCode::Char('a'), Action::Confirm),
        ];
        for (code, expected) in cases {
            assert_eq!(
                action_for(code, KeyModifiers::NONE),
                expected,
                "code: {code:?}"
            );
        }
    }
```

`candidate_row_label` already compiles (B5c), so this test fails only on the missing `Action` variants and key mappings.

**Step 2: Run test to verify it fails.**

```bash
cargo test -p qol --bin qol dev_console::tests::candidate_row_label_marks_host_default dev_console::tests::emu_keys_map_open_toggle_and_confirm 2>&1 | tail -20
```

Expected: FAIL to compile, `Action::OpenEmuDir`/`Action::ToggleArch`/`Action::Confirm` are not defined.

**Step 3: Write minimal implementation.**

Add the three variants to the `Action` enum, before `Ignore,`:

```rust
    OpenEmuDir,
    ToggleArch,
    Confirm,
```

Map the keys in `action_for`'s non-control `match code` block, before `_ => Action::Ignore,`:

```rust
        KeyCode::Char('o') | KeyCode::Char('O') => Action::OpenEmuDir,
        KeyCode::Char('t') | KeyCode::Char('T') => Action::ToggleArch,
        KeyCode::Char('a') | KeyCode::Char('A') => Action::Confirm,
```

`o`/`t`/`a` are unmapped in bare main and do not collide with the existing `c`/`C` (Copy) or any other binding. The existing `action_for_maps_keys` test does not assert on `o`/`t`/`a`, so it is unaffected.

Add `selected_candidate_mut` as the single plain-field form near `selected_emu_status`:

```rust
fn selected_candidate_mut(dash: &mut Dash) -> Option<&mut ImageCandidate> {
    dash.emu_cursor
        .checked_sub(emu_env_count(dash))
        .and_then(|index| dash.emu_candidates.get_mut(index))
}
```

Add the three arms to the top-level `match action` in `apply_action`, before `Action::Quit | Action::ReloadSelf | Action::Ignore => {}`:

```rust
        Action::OpenEmuDir => {
            if dash.view == View::Emu {
                open_emu_dir();
            }
        }
        Action::ToggleArch => {
            if dash.view == View::Emu {
                if let Some(candidate) = selected_candidate_mut(dash) {
                    candidate.arch = match candidate.arch {
                        crate::commands::emu::GuestArch::X86_64 => {
                            crate::commands::emu::GuestArch::Aarch64
                        }
                        crate::commands::emu::GuestArch::Aarch64 => {
                            crate::commands::emu::GuestArch::X86_64
                        }
                    };
                    candidate.arch_inferred = true;
                }
            }
        }
        Action::Confirm => {
            if dash.view == View::Emu {
                confirm_selected_candidate(dash);
            }
        }
```

The three new actions are off-Emu-page no-ops via the `View::Emu` gate. `ToggleArch`/`Confirm` are additionally no-ops off a candidate row, because `selected_candidate_mut` returns `None` when the cursor sits on an environment row (`checked_sub(emu_env_count(dash))` is `None`) or past the candidate list. Leave the three out of `preserves_arm` so they fall through to `false`; they mutate registry/dir/in-memory candidate state, so dropping the arm is correct.

Add the two side-effecting helpers near the other emu helpers (e.g. after `fire_emu_down`):

```rust
fn open_emu_dir() {
    let Some(dir) = emu_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    crate::host_facade::open_path(&dir);
}

fn confirm_selected_candidate(dash: &mut Dash) {
    let Some(qemu_img) = crate::commands::emu::find_on_path("qemu-img") else {
        return;
    };
    let Some(emu_toml) = emu_config_path() else {
        return;
    };
    let Some(candidate) = selected_candidate_mut(dash).map(|candidate| candidate.clone()) else {
        return;
    };
    if crate::commands::emu::register_image(&emu_toml, &candidate, &qemu_img).is_ok() {
        dash.pokes.emu = true;
    }
}
```

`open_emu_dir` calls the imported `emu_dir()` and `confirm_selected_candidate` calls the imported `emu_config_path()`, so the B1-era imports of those symbols stay used. `open_emu_dir` creates the dir if missing, then opens it via `host_facade::open_path` (B5a); on a headless host the per-OS opener no-ops. `confirm_selected_candidate` clones the candidate (its `&mut` borrow of `dash` ends before `dash.pokes.emu = true`, avoiding E0499), registers it, and on success sets `dash.pokes.emu = true`, which `flush_pokes` drains into `probes.emu.poke()` to trigger an immediate rescan. `ImageCandidate` derives `Clone` (B2), so `.clone()` works.

**Step 4: Run test to verify it passes, build, clippy.**

```bash
cargo test -p qol --bin qol dev_console::tests::candidate_row_label_marks_host_default dev_console::tests::emu_keys_map_open_toggle_and_confirm 2>&1 | tail -20
cargo build -p qol 2>&1 | tail -20
cargo clippy -p qol --all-targets -- -D warnings 2>&1 | tail -30
```

Expected: PASS. Both new tests green; the four existing `EmuState::Done(vec![...])` tests and `emu_status` unchanged and green; clippy clean (the three new actions are constructed in `action_for` and matched in `apply_action`; `selected_candidate_mut`/`open_emu_dir`/`confirm_selected_candidate`/`candidate_row_label` all have callers).

**Step 5: Commit.**

```bash
git add tools/qol-cli/src/dev_console.rs
git commit -m "feat(emu): wire TUI o/t/a keys and candidate-row labels"
```

## Verification (full CI-equivalent suite)

Run the full CI-equivalent gate after the last task lands:

```bash
cargo fmt --check
cargo clippy -p qol -p qol-config -p qol-tray --all-targets -- -D warnings
cargo test -p qol -p qol-config -p qol-tray
```
