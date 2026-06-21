# Remove App - Smarter, Safer Per-App Removal (Spec 1)

**Date:** 2026-06-21
**Status:** Design, revised after a 4-reviewer code-review board (verdict block -> addressed); pending implementation plan
**Plugin:** `plugins/plugin-removeapp`

## Goal

Make the existing per-app removal flow find more of an app's real leftovers and refuse to do damage, without ever sweeping a sibling app's data. Two themes: **owner-aware leftover discovery** and **consent-gated safety guards**.

## Context (current state)

`core/platform/macos.rs` discovers apps by reading the top level of `/Applications` and `~/Applications` and pulling `CFBundleIdentifier`/`CFBundleName` from `Info.plist`. Leftovers are found by **constructing exact paths** (`Library/<dir>/<name>` and `Library/<dir>/<bundle-id>`) across ~10 Library directories and keeping the ones that exist. Removal is one app at a time, Trash by default / Delete opt-in, refusing protected apps (System, `/Library/Apple`, managed-prefix bundle ids, non-writable paths). UI is picker -> confirm -> done.

Gaps this spec closes: exact-key matching misses `com.acme.foo.helper`-style siblings and login-item plists; nothing detects a running target or a Homebrew-managed app; nested/relocated apps are invisible.

## Scope

**In:** owner-aware fuzzy leftover discovery; running-app guard; Homebrew-cask guard; Spotlight (`mdfind`) discovery with fallback; reclaimed-space readout; matching headless CLI safety semantics.

**Out (deferred to Spec 2 - "Orphan/ghost sweep"):** the reverse index that finds leftovers with no owning installed app. Also out of this spec: per-leftover deselect, multi-select batch, size/last-used sorting, app icons, config surface, Linux impl, force-kill.

## Design

### 1. Owner-aware leftover discovery (pure rule + macOS enumeration)

Replace path-construction with **enumerate-then-classify**. The classification rule is pure, cross-platform, and lives in `core` (table-testable with no filesystem):

```rust
fn normalize_entry(entry: &str) -> &str {
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

A candidate entry `E` in a **bundle-id-keyed** Library dir is a leftover of target app `T` (bundle id `B_T`) iff `owner_of(E, all_installed_bids) == Some(B_T)`. This:
- includes `com.acme.foo` and `com.acme.foo.helper` (dot boundary),
- excludes `com.acme.foobar` (does not belong to `com.acme.foo`),
- excludes `com.acme.foo.shared` when a more-specific installed app (`com.acme.foo.shared`, or a longer prefix) owns it - that data is the sibling's.

**Hybrid name/bundle-id dirs** (`Application Support`, `Caches`, `Logs`) keep the existing exact case-insensitive app-name match and also classify bundle-id-shaped entries with `owner_of`. This preserves current hits such as `Caches/com.acme.foo` while still refusing fuzzy app-name prefixes. There is no prefix matching on human app names.

macOS `scan` changes internally: for each Library dir, read its entries and classify, instead of joining guessed paths. The set of Library dirs and their key mode is macOS knowledge and stays in `platform/macos.rs`. `scan` now takes the discovered inventory as an explicit argument (see *App-universe boundary*).

**Library-dir classification.** Each dir is one of: *bundle-keyed* (entries are bundle ids -> `owner_of`), *name-keyed* (entries are human app names -> exact case-insensitive match only), *hybrid* (both forms appear), or *shared* (namespace co-owned by an app group -> exact bundle-id match only, never prefix):

| Library dir | Key mode |
|---|---|
| `Preferences/<bid>.plist` | bundle-keyed |
| `Containers/<bid>` | bundle-keyed |
| `HTTPStorages/<bid>` | bundle-keyed |
| `WebKit/<bid>` | bundle-keyed |
| `Saved Application State/<bid>.savedState` | bundle-keyed |
| `LaunchAgents/<bid>.plist` | bundle-keyed |
| `Application Support/<name\|bid>` | hybrid |
| `Caches/<name\|bid>` | hybrid |
| `Logs/<name\|bid>` | hybrid |
| `Group Containers/<group-bid>` | **shared - exact only** (group containers are co-owned; never prefix-match) |

**Fail-closed rules (blocker-2, mediums 1/5):**
- *Incomplete inventory:* dot-boundary (non-exact) matching depends on a complete inventory to exclude siblings. When that is uncertain - Spotlight unavailable *and* a discovered app sits outside the dir-walk - the "never sweeps a sibling's data" guarantee cannot hold for prefix matches. So **every non-exact (fuzzy) leftover is Trash-only and never hard-deleted, even with `--force`**; only the app bundle and *exact* bundle-id/name leftovers are Delete-eligible. A misclassification's blast radius is bounded to a recoverable Trash move.
- *Shared namespaces:* `Group Containers` and any shared dir use exact bundle-id match only.
- *Missing bundle id:* an app with no `CFBundleIdentifier` gets the app bundle plus *exact name-keyed* leftovers only - no bundle-id classification, no prefixing.

**App-universe boundary (medium-2):** classification needs `all_installed_bids`. `scan` receives the inventory as an explicit `&[InstalledApp]` planning context rather than re-discovering inside the platform, so `with_roots` fixtures stay hermetic and real `mdfind` never leaks into unit tests.

### 2. Spotlight discovery with fallback

`installed_apps` additionally runs `mdfind "kMDItemContentType == 'com.apple.application-bundle'"` to catch apps nested in subfolders or installed outside the two known dirs, deduped by canonical path against the existing dir-walk. If `mdfind` is absent or returns nothing (Spotlight disabled), the dir-walk result stands. No signature change; failure is silent.

### 3. Safety guards (consent-gated: warn + one-key act)

Both guards can apply at once, so they are a struct, not an enum:

```rust
pub struct CaskToken(String); // parsed at the boundary: ^[a-z0-9][a-z0-9+._-]*$

pub enum CaskStatus {
    Managed(CaskToken),  // confidently the cask's own app, unambiguous
    NotManaged,          // brew present, app is not a cask artifact
    Unavailable(String), // brew missing/broken/timeout/ambiguous - reason shown
}

pub struct Guards {
    pub running: bool,
    pub cask: CaskStatus,
}
```

New `AppPlatform` methods (stubbed on non-macOS):

```rust
fn is_running(&self, app: &InstalledApp) -> bool;
fn quit(&self, app: &InstalledApp) -> Result<()>;
fn cask_status(&self, app: &InstalledApp) -> CaskStatus;
fn brew_uninstall(&self, token: &CaskToken) -> Result<()>;
```

- **Running:** `is_running` matches `NSWorkspace.runningApplications` by bundle id (objc2); `quit` calls `NSRunningApplication.terminate()` - graceful Cmd-Q, never force in v1.
- **Homebrew (`cask_status`):** deterministic and fallible.
  - **Resolution is trusted-first (high-2):** `/opt/homebrew/bin/brew`, then `/usr/local/bin/brew`, then `PATH` last. A hijacked `PATH` must not pick the brew binary ahead of the canonical prefixes. Invoke via `std::process::Command` argv only - never a shell string - with `HOMEBREW_NO_AUTO_UPDATE=1` and a subprocess timeout. Spotlight uses absolute `/usr/bin/mdfind`.
  - Build a basename->`CaskToken` map from `brew info --json=v2 --installed`, resolving each cask's `app` artifact (`target:`-aware), then match the target app's **canonical-path** basename. Appdir-independent (works for `~/Applications` fallback installs).
  - **Tri-state (high-3, high-4):** exactly one unambiguous cask -> `Managed(token)`; brew present, no match -> `NotManaged`; brew missing/non-zero/timeout/malformed JSON, **or** the basename is ambiguous (maps to >1 cask, or >1 installed app shares it) -> `Unavailable(reason)`. A lookup failure must never collapse to `NotManaged`.
  - `[B] brew uninstall` is offered **only** for `Managed`. `brew_uninstall(&CaskToken)` runs `brew uninstall --cask -- <token>` (argv + `--` guard), then trashes the remaining non-bundle leftover paths.

**Determinism (validated on 27 installed casks).** Basename is brew's *own* key: the install receipt records each cask's app by basename + zap globs and **never an `/Applications` path** (`installed_as: n/a`), so there is no stricter path oracle to prefer. Empirical result: 1:1 token map, zero basename collisions, zero multi-path ambiguity; 16/16 apps that exist in an appdir matched their cask; 7 no-`.app` casks (fonts/pkg/CLI) skipped with no false guard. The only non-matches were 4 stale, Caskroom-only cask records whose bundle exists nowhere on disk - un-pickable, so the guard never fires for them. Residual: two distinct apps sharing an identical bundle filename now resolve to `Unavailable` (no `[B]`, normal trash flow), so a wrong `brew uninstall` is impossible.

`core` exposes `fn guards(plat, app) -> Guards` computed before the Confirm screen.

### 4. UI changes

- **Confirm screen** gains a guard banner driven by `Guards` (state machine in section 6):
  - running -> `[Q] quit & continue`,
  - `Managed` cask -> `[B] brew uninstall` (removes the bundle via brew, **then trashes the remaining leftover paths** so brew stays consistent *and* user-library cruft is cleaned),
  - `Unavailable` cask -> non-blocking advisory line ("couldn't confirm Homebrew - check manually"); falls through to the normal path,
  - `[T] trash anyway` is the **only** waive (forces Trash), `esc` back. `[enter]` is **rejected** while any guard is unresolved.
- **Done screen** adds "Freed N GB". **Freed bytes** = sum of pre-removal sizes of paths *actually* removed; a brew-handled bundle counts only after `brew uninstall` succeeds; partial failures report `freed = sum(removed)` with failures listed. JSON: `{ removed, failed, freed_bytes, brew: <token|null> }`.

### 5. Headless CLI changes

The terminal path must enforce the same guard contract as the picker. `removeapp remove <query>` computes `plan` and `guards` before acting, even with `--yes`.

- Non-interactive mode means `--yes` is present or stdin is not a terminal.
- `--dry-run` prints the plan and guard state, then exits without acting.
- `--yes` answers the ordinary confirmation prompt only. It does **not** silently waive running-app or Homebrew guards.
- If `running` is true, interactive mode offers quit-and-recheck before removal. Non-interactive mode requires `--quit`; if the app remains running after quit, removal fails unless `--trash-anyway` is present.
- If `cask` is `Managed`, interactive mode offers brew uninstall; non-interactive requires `--brew` or `--trash-anyway`. `Unavailable` prints an advisory and proceeds via the normal path (no flag required); `NotManaged` is silent.
- The brew path runs `brew uninstall --cask <token>`, then trashes the remaining non-bundle leftover paths from the plan. The app bundle itself is handled by Homebrew and must not be passed to `remove_paths`.
- The normal trash/delete path is still protected by `is_protected`; `--force` changes Trash to hard delete only on that normal path.
- With no guard tripped (`running == false` and `NotManaged`/`Unavailable`): prompt unless `--yes`, then two-phase remove with Trash by default.

### 6. Removal contract and guard state machine

**Two-phase normal removal (blocker-1).** `remove_paths` today continues after a per-path failure, so a failed app-bundle removal alongside successful leftover removal would strand a half-uninstalled app with its data gone. Normal removal is two-phase:
1. Remove the **app bundle** first. If it fails, **abort** - touch no leftover, surface the error.
2. Only on bundle success, remove the leftovers (collecting per-path failures as today).
The brew path keeps the inverse invariant: brew owns the bundle; if `brew uninstall` fails, leftovers are left untouched.

**Canonical identity (high-5).** At discovery each `InstalledApp` carries the **canonicalized** bundle path. Protection (`is_protected`), dedupe, cask matching, and mutation all use the canonical path. If canonicalization fails, the app is treated as protected (fail-closed). An app bundle that is itself a symlink is refused for destructive action.

**Guard state machine (high-1).** Only `running == false` with `cask in {NotManaged, Unavailable, Managed-resolved-via-[B]}` may proceed to mutation.
- UI: while any guard is **unresolved**, `[enter]` is a no-op with a hint. `[Q]` resolves running; `[B]` resolves a `Managed` cask; `[T] trash anyway` is the **only** key that intentionally waives unresolved guards and forces Trash (never hard delete).
- CLI mirrors this: unresolved guards require the explicit `--quit` / `--brew` / `--trash-anyway`; `--yes` waives only the ordinary prompt.

**Execution-boundary recheck (high-6, medium-6).** Immediately before mutation (UI and CLI), re-evaluate `is_running` and re-`lstat` each planned path - verifying its root, name, and classification are unchanged and no ancestor became a symlink. If running flipped or any path's identity changed, return to guard resolution instead of mutating. This closes the confirm->execute TOCTOU window (a mitigation, not a same-user race guarantee - see Risks).

### 7. Architecture (qol-arch-code)

Pure matching (`belongs_to`, `owner_of`, `guards` assembly) lives in `core`. Everything OS-specific (NSWorkspace, `mdfind`, `brew`, Library dir list) stays behind `AppPlatform` + `platform/macos.rs`. `platform/linux.rs` and `platform/windows.rs` get typed stubs: `is_running` -> `false`, `quit`/`brew_uninstall` -> typed `Err`, `cask_token` -> `None`. No `#[cfg]` in business logic; no `compile_error!`.

## Data flow

picker or CLI (search over Spotlight+dir-walk apps) -> on select/query: `scan` (enumerate+classify -> `RemovalPlan`) **and** `guards` -> user resolves guards via UI keys or explicit CLI flags -> `remove_paths` (brew path: bundle handled by brew, rest by trash) -> Done / JSON output with freed bytes.

## Error handling

- `mdfind` missing/empty -> fall back to dir-walk, no error.
- `quit` fails (app refuses) -> banner shows "couldn't quit <app>"; user retries or `[T]`.
- `brew_uninstall` non-zero -> show stderr tail; app left in place; user can `[T]`.
- Subprocess hygiene (medium-7): `brew`/`mdfind` run with a timeout; stdout and stderr captured separately (structured JSON stays stdout-only); displayed stderr is byte-capped and ANSI/control-stripped; `mdfind` output parsed line-wise, non-`.app` lines ignored.
- `brew` absent or no artifact-basename match -> empty map -> no brew banner.
- CLI guard refusal exits non-zero with an actionable stderr message naming the required explicit flag.
- Missing Library dir during enumeration -> skip (as today).
- Protected target -> existing refusal, unchanged.

## Testing

- **Pure rule (table-driven):** `belongs_to`/`owner_of` over exact match, dot-boundary helper, `foobar` non-match, sibling-owned exclusion (longest-prefix wins), `.plist`/`.savedState` suffix stripping, no-owner.
- **Classification (tempdir fixtures):** create `Foo.app` + sibling `Bar.app` (`com.acme.foo.bar`) and seed leftovers; assert Foo's plan includes `com.acme.foo.helper`, excludes `com.acme.foobar` and `com.acme.foo.bar.*`.
- **Guards (table-driven over FakePlat flags):** `(running?, cask?)` -> expected `Guards`.
- **Removal:** extend FakePlat tests for the brew path (bundle skipped, remaining paths trashed).
- **Two-phase abort (blocker-1):** bundle-removal failure leaves every leftover untouched; bundle success then proceeds to leftovers.
- **Guard state machine (high-1):** `enter` is rejected while running or `Managed` is unresolved; `[T]` waives and forces Trash; CLI requires `--quit`/`--brew`/`--trash-anyway`.
- **Cask tri-state (high-3/4):** fixture `brew info --json=v2` -> `Managed` (unique), `NotManaged` (no match), `Unavailable` (missing brew, malformed JSON, basename maps to >1 cask, >1 installed app shares the basename); `CaskToken` parsing rejects `-`-leading / illegal tokens.
- **Canonical/protection (high-5):** symlinked app bundle is refused for destructive action; canonicalization failure is treated as protected.
- **Fuzzy is Trash-only (blocker-2):** non-exact leftovers are never hard-deleted even with `--force`.
- **Headless CLI guards:** table-test `remove` behavior for running only, cask only, both guards, `--yes`, `--dry-run`, `--force`, `--quit`, `--brew`, `--trash-anyway`, quit failure, and brew failure.
- **Cask map (pure, table-driven):** parse a fixture `brew info --json=v2` payload into the basename->token map and assert membership for matched / unmatched / appdir-relocated apps, including a cask with multiple `.app` artifacts.
- **Not unit-tested (per no-test-for-thin-wrappers):** NSWorkspace, `mdfind`, and the `brew` subprocess invocation itself - thin platform wrappers. The JSON parse above is the testable part.

## Open risks

- **Brew filename collision** - resolved: ambiguous basenames become `CaskStatus::Unavailable`, so `[B]` is never offered for a non-brew app; the user gets the normal trash flow.
- **Same-user filesystem races (TOCTOU)** - the confirm->execute window is mitigated by an execution-boundary recheck (re-`lstat`, re-`is_running`), **not eliminated**. A deliberate same-uid race is out of scope; Trash-by-default keeps it recoverable. *(Calibration: re-`lstat` revalidation, not full fd-relative `openat` operations - the 90% mitigation for a personal single-user tool.)*
- **Brew `Unavailable`** - shown as a non-blocking advisory, **not** a hard block. *(Calibration vs the review's "require explicit override": a slow/broken brew should not wedge a removal; the user is warned and proceeds via the normal path.)*
- **NSWorkspace objc2 FFI** - graceful terminate only; no force-kill path in v1.
- **Spotlight disabled** - dir-walk fallback covers the common case; fuzzy matching is fail-closed (Trash-only) when the inventory may be incomplete.
- **Name-keyed dir collisions** - mitigated by keeping name matching exact (no prefixing); shared namespaces (Group Containers) are exact-only.
