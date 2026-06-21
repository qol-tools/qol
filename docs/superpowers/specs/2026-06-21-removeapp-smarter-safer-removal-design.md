# Remove App - Smarter, Safer Per-App Removal (Spec 1)

**Date:** 2026-06-21
**Status:** Design, pending implementation plan
**Plugin:** `plugins/plugin-removeapp`

## Goal

Make the existing per-app removal flow find more of an app's real leftovers and refuse to do damage, without ever sweeping a sibling app's data. Two themes: **owner-aware leftover discovery** and **consent-gated safety guards**.

## Context (current state)

`core/platform/macos.rs` discovers apps by reading the top level of `/Applications` and `~/Applications` and pulling `CFBundleIdentifier`/`CFBundleName` from `Info.plist`. Leftovers are found by **constructing exact paths** (`Library/<dir>/<name>` and `Library/<dir>/<bundle-id>`) across ~10 Library directories and keeping the ones that exist. Removal is one app at a time, Trash by default / Delete opt-in, refusing protected apps (System, `/Library/Apple`, managed-prefix bundle ids, non-writable paths). UI is picker -> confirm -> done.

Gaps this spec closes: exact-key matching misses `com.acme.foo.helper`-style siblings and login-item plists; nothing detects a running target or a Homebrew-managed app; nested/relocated apps are invisible.

## Scope

**In:** owner-aware fuzzy leftover discovery; running-app guard; Homebrew-cask guard; Spotlight (`mdfind`) discovery with fallback; reclaimed-space readout.

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

**Name-keyed** dirs (`Application Support/<Name>`, `Caches/<Name>`, `Logs/<Name>`) keep **exact** case-insensitive name matching (no prefixing) - names collide too easily for fuzzy matching to be safe.

macOS `scan` changes internally: for each Library dir, read its entries and classify, instead of joining guessed paths. The set of Library dirs and which are bundle-id-keyed vs name-keyed is macOS knowledge and stays in `platform/macos.rs`. `scan`'s signature is unchanged.

### 2. Spotlight discovery with fallback

`installed_apps` additionally runs `mdfind "kMDItemContentType == 'com.apple.application-bundle'"` to catch apps nested in subfolders or installed outside the two known dirs, deduped by canonical path against the existing dir-walk. If `mdfind` is absent or returns nothing (Spotlight disabled), the dir-walk result stands. No signature change; failure is silent.

### 3. Safety guards (consent-gated: warn + one-key act)

Both guards can apply at once, so they are a struct, not an enum:

```rust
pub struct Guards {
    pub running: bool,
    pub cask: Option<String>, // Homebrew cask token, if confidently detected
}
```

New `AppPlatform` methods (stubbed on non-macOS):

```rust
fn is_running(&self, app: &InstalledApp) -> bool;
fn quit(&self, app: &InstalledApp) -> Result<()>;
fn cask_token(&self, app: &InstalledApp) -> Option<String>;
fn brew_uninstall(&self, token: &str) -> Result<()>;
```

- **Running:** `is_running` matches `NSWorkspace.runningApplications` by bundle id (objc2); `quit` calls `NSRunningApplication.terminate()` - graceful Cmd-Q, never force in v1.
- **Homebrew:** `cask_token` is **deterministic**, not best-effort. Build a basename->token map from `brew info --cask --json=v2 --installed` (each cask's `app` artifact filename), then test the target's actual `path` basename against it. This is **appdir-independent** - it does not assume `/Applications`, so a cask installed to `~/Applications` (brew's fallback when `/Applications` is not writable) still matches against the path `installed_apps` already discovered. `brew` absent -> empty map -> correctly no nudge. `brew_uninstall` shells `brew uninstall --cask <token>`.

`core` exposes `fn guards(plat, app) -> Guards` computed before the Confirm screen.

### 4. UI changes

- **Confirm screen** gains a guard banner driven by `Guards`:
  - running -> `[Q] quit & continue` (primary), then re-checks and clears,
  - cask -> `[B] brew uninstall` (runs `brew uninstall --cask`, which removes the bundle, **then trashes the remaining leftover paths** so brew stays consistent *and* user-library cruft is cleaned),
  - always -> `[T] trash anyway`, `[enter] confirm`, `esc` back.
- **Done screen** adds "Freed N GB" alongside removed/failed counts.

### 5. Architecture (qol-arch-code)

Pure matching (`belongs_to`, `owner_of`, `guards` assembly) lives in `core`. Everything OS-specific (NSWorkspace, `mdfind`, `brew`, Library dir list) stays behind `AppPlatform` + `platform/macos.rs`. `platform/linux.rs` and `platform/windows.rs` get typed stubs: `is_running` -> `false`, `quit`/`brew_uninstall` -> typed `Err`, `cask_token` -> `None`. No `#[cfg]` in business logic; no `compile_error!`.

## Data flow

picker (search over Spotlight+dir-walk apps) -> on select: `scan` (enumerate+classify -> `RemovalPlan`) **and** `guards` -> Confirm renders plan + banner -> user resolves guards via `[Q]`/`[B]` or proceeds via `[T]`/`[enter]` -> `remove_paths` (brew path: bundle handled by brew, rest by trash) -> Done with freed bytes.

## Error handling

- `mdfind` missing/empty -> fall back to dir-walk, no error.
- `quit` fails (app refuses) -> banner shows "couldn't quit <app>"; user retries or `[T]`.
- `brew_uninstall` non-zero -> show stderr tail; app left in place; user can `[T]`.
- `brew` absent or no artifact-basename match -> empty map -> no brew banner.
- Missing Library dir during enumeration -> skip (as today).
- Protected target -> existing refusal, unchanged.

## Testing

- **Pure rule (table-driven):** `belongs_to`/`owner_of` over exact match, dot-boundary helper, `foobar` non-match, sibling-owned exclusion (longest-prefix wins), `.plist`/`.savedState` suffix stripping, no-owner.
- **Classification (tempdir fixtures):** create `Foo.app` + sibling `Bar.app` (`com.acme.foo.bar`) and seed leftovers; assert Foo's plan includes `com.acme.foo.helper`, excludes `com.acme.foobar` and `com.acme.foo.bar.*`.
- **Guards (table-driven over FakePlat flags):** `(running?, cask?)` -> expected `Guards`.
- **Removal:** extend FakePlat tests for the brew path (bundle skipped, remaining paths trashed).
- **Cask map (pure, table-driven):** parse a fixture `brew info --json=v2` payload into the basename->token map and assert membership for matched / unmatched / appdir-relocated apps.
- **Not unit-tested (per no-test-for-thin-wrappers):** NSWorkspace, `mdfind`, and the `brew` subprocess invocation itself - thin platform wrappers. The JSON parse above is the testable part.

## Open risks

- **Brew filename collision** - a non-brew app sharing a cask artifact's exact basename would get a brew nudge. Negligible in practice, and `[T] trash anyway` covers it. (Detection itself is deterministic; the appdir trap is handled by matching the discovered path's basename, not a hardcoded `/Applications`.)
- **NSWorkspace objc2 FFI** - graceful terminate only; no force-kill path in v1.
- **Spotlight disabled** - mitigated by dir-walk fallback.
- **Name-keyed dir collisions** - mitigated by keeping name matching exact (no prefixing).
