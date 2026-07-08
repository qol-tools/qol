# qol-monorepo: Multiple-Sources-of-Truth & Architectural Grit Findings

**Date:** 2026-07-06
**Scope:** Whole monorepo (`libs/`, `apps/qol-tray`, `tools/qol-cli`, `plugins/*`)
**Method:** Direct manual grep/read (no Agent-tool fan-out - this repo's standing rule after repeated cost incidents). Re-verified the 2026-07-01 catalogue against current `main` after ~80 commits landed in between, then swept the recent daemon-lifecycle/socket-fd-handoff churn that was never audited for this angle.

This is a findings list, not an implementation plan. Nothing below has been fixed or scoped into tasks yet - pick items and design the actual fix per item; several are in unrelated subsystems and don't share one architecture.

---

## Resolved since 2026-07-01 (do not re-propose)

**Platform-support computed two ways, opposite answers on `platforms: []`.**
`tools/qol-cli/src/workspace.rs`'s divergent `supports_host` free function is gone entirely - deleted as part of extracting build/workspace logic into the new `libs/qol-dev-build` crate. That crate depends directly on `qol-plugin-api` and calls `manifest.plugin.supports_current_platform()`, confirmed to be a thin wrapper (`libs/qol-plugin-api/src/manifest/schema.rs:278`) around the one canonical free function (`libs/qol-plugin-api/src/manifest/mod.rs:32`). Likely an incidental side effect of the qol-dev-build consolidation, not a deliberate fix - still counts.

---

## Confirmed open findings

### 1. Health-snapshot response DTO hand-duplicated across an HTTP boundary (new)

The exact same response shape is independently hand-typed on both sides of the qol-tray↔qol-cli dev-server boundary, under different names, in the same language, in the same Cargo workspace:

- **Server** - `apps/qol-tray/src/plugins/daemon_health.rs`: `PluginRuntimeStatus` (enum, `#[serde(tag = "state", rename_all = "snake_case")]`, variants `NotExpected` / `AutostartBlocked` / `OnDemand{pid}` / `Down{consecutive_failures,suppressed}` / `Probation{pid,consecutive_failures}` / `Stable{pid}`), `HealthSnapshot` (struct: `tick, process_pid, role, bind_port, daemon_autostart_held, generation_id, plugins: Vec<PluginHealth>`), `PluginHealth` (`plugin_id, status`).
- **Client** - `tools/qol-cli/src/dev_server.rs`: `PluginDaemonStatus` (byte-identical variants/fields/serde tag), `PluginHealthSnapshot` (subset of `HealthSnapshot`'s fields), `PluginHealthRow` (identical shape to `PluginHealth`).

Unlike a JS-frontend/Rust-backend boundary, both sides here are Rust in the same workspace - this could be a shared DTO type (in `qol-conventions` or a new tiny crate) at zero cross-language cost, but isn't. A renamed field or new enum variant on the server won't fail to compile on the client - it'll silently mis-parse or drop data at runtime instead (no `deny_unknown_fields` on either side).

**Better path:** the client's `PluginHealthSnapshot` is a strict field-subset of the server's `HealthSnapshot` (drops `process_pid`, `role`, `bind_port`, `generation_id`) - there's no reason to keep a second, narrower struct. Move `PluginRuntimeStatus`/`HealthSnapshot`/`PluginHealth` to a crate both already depend on (`qol-conventions` is the natural fit - both `apps/qol-tray` and `tools/qol-cli` already use it), and have `dev_server.rs` deserialize straight into the real type, deleting its own copies. The client simply won't touch the fields it doesn't need; nothing is lost by not having its own trimmed mirror.

### 2. Dev HTTP route strings hardcoded independently, client/server (grown in scope)

`apps/qol-tray/src/features/plugin_store/server/dev_handlers.rs::routes()` is byte-identical to 2026-07-01: 5 literal path strings (`/dev/reload`, `/dev/reload/{plugin_id}`, `/dev/recompile-self`, `/dev/worktrees`, `/dev/active-worktree`) next to 2 calls to `qol_conventions::DEV_RESTART_PREBUILT_ROUTE` / `DEV_PROMOTE_GENERATION_ROUTE`. Neither file has been touched since.

The same unhoisted-literal pattern has since spread to at least 4 more sibling handler files - `dev_health_handlers.rs` (`/dev/plugin-health`), `dev_link_handlers.rs` (`/dev/links`, `/dev/links/{id}`), `dev_state_handlers.rs` (`/dev/discovery-state`), `meta_handlers.rs` (`/dev/enabled`) - each matched by an independently-typed literal in `tools/qol-cli/src/dev_server.rs` (`dev_links_url`, `dev_discovery_url`, `plugin_health_url`, `dev_health_url`, etc). Zero additional route constants have been hoisted since 2026-07-01 despite the surface area roughly doubling.

**Why "just hoist more constants" probably won't stick:** the pattern (2 routes already hoisted into `qol_conventions`) has existed since before 2026-07-01 and nobody's extended it to any of the ~10 routes added since - hoisting more constants by hand is exactly the fix that already failed to get adopted once. **Better path:** pair the fix with something that makes drift fail loudly instead of relying on developers remembering to reuse a constant next time - e.g. an integration test that boots the dev server and asserts every URL `dev_server.rs` builds actually resolves (catches a renamed/removed server route as a test failure), or a single shared table of `(name, path)` pairs that both the `Router` builder and the client's URL functions iterate over, so there's structurally one list instead of a convention to remember.

### 3. Hotkey string `"delete"` maps to two different physical keycodes

**This is not simply "pick one" - one side is internally correct, the other side conflates two concepts it shouldn't.**

`libs/qol-hotkeys/src/grammar.rs` already has both `NamedKey::Backspace` and `NamedKey::Delete` as **separate** variants (line 97: `"backspace" => NamedKey::Backspace`; line 98: `"delete" | "del" => NamedKey::Delete`). `macos_keycode.rs::named_to_keycode` resolves them to different physical keys: `NamedKey::Backspace => DELETE` (`0x33`, the everyday Mac "delete" key that behaves like backspace), `NamedKey::Delete => FORWARD_DELETE` (`0x75`, `fn+delete` / the PC-style forward-delete). This matches the standard cross-platform convention (Backspace deletes left, Delete deletes right) and is covered by an existing test asserting `code("delete") == Some(FORWARD_DELETE)`. This path is used by `apps/qol-tray/src/hotkeys/{parser.rs, capture/binding.rs, capture/platform/macos.rs}` - the shortcuts/hotkey-capture feature.

The standalone `libs/qol-hotkeys/src/macos_keycode.rs::parse_key` (line 91, the **only** entry point used by `plugins/plugin-keyremap/src/remap.rs`) instead **conflates** the two: line 133, `"delete" | "backspace" => Some(DELETE)` (`0x33`) - treating them as synonyms - while requiring a separate string `"forwarddelete"` (line 134) to reach `0x75`.

**Read on which side is the bug:** `parse_key` is the outlier. The rest of the same crate already encodes the "Backspace ≠ Delete" distinction correctly and has a test enforcing it; `parse_key` re-implements a second, informal string table that quietly drops that distinction, most likely written by someone thinking of "the key labeled delete on my Mac keyboard" without checking that the crate already has a canonical answer two files away.

**Better path:** fix `parse_key` to split `"backspace" => DELETE` / `"delete" => FORWARD_DELETE`, matching `named_to_keycode` - or better, have `parse_key` delegate to `grammar::parse_key` + `named_to_keycode` instead of hand-rolling a second table, so there is structurally only one mapping to drift from.

**Compatibility caveat - do not skip this:** this is a **user-facing behavior change** for plugin-keyremap. Anyone who already has `"delete"` configured as a remap target today gets `0x33`; after the fix they'd silently get `0x75` on next daemon restart - a different physical key than what they set up. Worth a release-note callout, and worth checking whether `qol-migrations` should detect and flag/rewrite existing keyremap configs that reference bare `"delete"`.

### 4. Version-string parsing disagrees on a `v`/`V` prefix

`libs/qol-migrations/src/lib.rs:288` `parse_semver("v1.2.3")`: splits on `.` first (`"v1"`, `"2"`, `"3"`), then for `"v1"` splits again on the first non-digit char and takes `.next()`, which is the segment *before* that split point - i.e. `""`. Major silently becomes `0`. `apps/qol-tray/src/version.rs:36` `Version::parse` strips `['v','V']` first via `trim_start_matches` and is correct (has a test explicitly covering `"v1.2.3"` and `"v0.1.0"`). Both functions are private to their own crate - `parse_semver`'s only 2 callers are internal to `lib.rs` (`reject_if_below_oldest_supported`, `compare_semver`).

**This may not be theoretical - there's already a workaround for what looks like the same symptom.** `reject_if_below_oldest_supported` (`lib.rs:269`) contains this:

```rust
let installed_major = parse_semver(&installed).0;
if installed_major == 0 && compare_semver(host_version, OLDEST_SUPPORTED) >= 0 {
    log::warn!(
        "[qol-migrations] version.txt contains {installed} (major == 0, the buggy lib stamp); \
         host {host_version} is current. Treating as the env!CARGO_PKG_VERSION bug and \
         overwriting with host version after this run."
    );
    return Ok(());
}
```

This is a runtime safety-valve that already exists specifically to tolerate `installed_major == 0` when the host is otherwise current, log-labeled as a known "buggy lib stamp" / "env!CARGO_PKG_VERSION bug." That's *exactly* the symptom a `v`-prefixed `version.txt` would produce through today's `parse_semver`. Not fully confirmed this is the *same* root cause (didn't trace how `version.txt` actually gets stamped, or git-blame this workaround to see what it was originally written against) - but it's a strong enough coincidence to check before treating this as a low-priority style fix. If it is the same cause: fixing `parse_semver` to strip the prefix makes this class of false-positive rarer, but the workaround itself is still worth keeping as defense-in-depth for genuinely corrupt version files, not removing.

### 5. "Safe identifier" rule reimplemented independently in 3 places

- Canonical: `libs/qol-plugin-api/src/manifest/validation/command_rules.rs:22` `is_valid_command_basename` - rejects null bytes and leading/trailing whitespace (`value.trim() == value`), plus the shared shape check (non-empty, len ≤ 64, no leading `-`, charset `[A-Za-z0-9_-]`).
- `apps/qol-tray/src/paths.rs:81` `is_safe_path_component` and `apps/qol-tray/src/shortcuts/validation.rs:14` `validate_id` both drop the null-byte/whitespace checks, otherwise identical shape.

(The `libs/qol-migrations` copies of this same rule were already unified internally in an earlier simplification pass - that dedup didn't touch these 3.)

### 6. Malformed-hex-color fallback differs 3 ways inside plugin-lights

- `src/daemon/ws.rs:181` `parse_hex` - bad/missing byte falls back to `0xff` per channel (toward white), no `#` stripping.
- `src/daemon/state.rs:340,349` `parse_color`/`parse_hex_pair` - falls back to `0x00` per channel (toward black), does strip a leading `#`.
- `src/config/validation.rs:57` `is_hex_color` - strict len==6 reject, no `#` handling, validation-only.

`libs/qol-color::parse_hex_color` is the correct `Option`-returning version, already used by plugin-alt-tab. plugin-lights' `Cargo.toml` has no `qol-color` dependency at all.

**Call-site tracing changes the priority here - these two fallbacks are not equally reachable:**

- `ws.rs::parse_hex` is called from `parse_pending()`, which handles `cmd.hex` straight off an incoming WebSocket message: `serde_json::from_str::<WsCommand>(text)` with **no validation gate before `parse_hex` runs.** Any client that can reach the plugin's local WS port (the tray UI today, but nothing stops anything else on the box) can send garbage hex and get a silent, wrong color (`0xffffff`) instead of a rejected command.
- `state.rs`'s 3 call sites all read `config.live_color_hex` / `preset.color_hex`, i.e. values that (assuming `is_hex_color` actually gates every config-write path, not verified end-to-end) should already be valid by the time they're parsed - making that fallback more of a defensive last-resort than a live bug.

**Better path:** prioritize `ws.rs`. Use `qol_color::parse_hex_color` in `parse_pending()` and have it return `None` (drop the command, same as an unrecognized `cmd.kind` already does) instead of silently substituting white. That's a real input-validation gap on untrusted network input, not just a style inconsistency - worth ranking above the `state.rs` copies.

### 7. Settings/URL opener hand-rolled 8+ times, with confirmed behavioral drift

`plugins/qol-shot/src/platform/macos/system.rs` alone has 3 separate call sites (lines 33, 110, 122) independently doing `Command::new("open")`, plus `platform/linux.rs:373` (`xdg-open`). Also `plugin-template/src/platform/{linux,macos}.rs`, `plugin-pointz/src/platform/{linux,macos}.rs` (confirmed: `let _ = std::process::Command::new(...)` - silently discards the result, no error propagation, unlike the others), and `plugin-os-themes/src/cursor/platform/linux/mod.rs` (at least names a local `const OPENER = "xdg-open"`). The `open` crate (v5) is already a dependency of `apps/qol-tray` and `plugin-alt-tab` - unused by any of these.

**Confirmed broader still**: plugin-pointz also has `src/platform/windows.rs`, and it has the *same* silent-discard bug: line 2, `let _ = std::process::Command::new("cmd")...`. So pointz alone hand-rolls this on all 3 platforms with the same flaw, not 2. Swapping pointz's 3 platform files to the `open` crate (already proven elsewhere in the workspace) fixes all 3 at once.

### 8. `has_process_focus`: two full parallel platform-strategy subsystems

Not just one duplicated function - two complete 4-file platform splits for the same concept:

- `libs/qol-gpui/src/platform/{mod.rs:43, macos.rs:48, linux.rs:68, fallback.rs:27}` - used internally by `qol-gpui`'s own `ghost.rs:310`.
- `libs/qol-plugin-daemon/src/focus/{mod.rs:7, platform/macos.rs:7, platform/linux.rs:13, platform/fallback.rs:5}`

**Reconfirmed independently this pass, not just carried from memory**: a fresh repo-wide grep for `has_process_focus` across every crate found only the qol-plugin-daemon module's own internal wiring (`focus/mod.rs` calling its own `platform::has_process_focus()`, and the 3 platform submodules re-exporting into it) - **zero callers anywhere outside `qol-plugin-daemon` itself.** This is why `cargo clippy`/`dead_code` never flagged it: Rust doesn't lint unused `pub` items in a library crate, since they're presumptively public API for external consumers - even other crates in the same workspace that never actually import them.

**Better path, different from "merge the two":** this repo's own `CLAUDE.md` already says "remove unused code or gate it behind a feature flag." Given zero consumers repo-wide, the simplest correct fix for `qol-plugin-daemon::focus` is likely to **delete the whole module**, not merge it with `qol-gpui`'s copy - merging would keep dead code alive under a new name. Only keep it if there's a concrete near-term plan to wire a daemon to it (worth asking, not assuming).

### 9. Notification dispatch duplicated

`plugins/plugin-cli-sessions/src/notify.rs` (lines 38, 49) and `plugins/plugin-ide-checkout/src/main.rs` (`send_notification`/`send_osascript_notification`, lines 77-109) each independently shell out to `osascript` + `notify-send`.

**Escaping diffed this pass - no security concern.** Both use the literal same logic: `s.replace('\\', "\\\\").replace('"', "\\\"")`. No injection-risk drift between the two; this really is "just" duplication, not a divergent-correctness bug.

**Real difference is robustness, not escaping.** `plugin-cli-sessions::send()` picks `osascript` vs `notify-send` at **compile time** via `#[cfg(target_os = "macos")]`, and discards the result: `let _ = spawn_platform(...)`. `plugin-ide-checkout::send_notification` tries `osascript` first **at runtime regardless of OS**, falls back to `notify-send` if that fails, and falls back to `println!` if both fail. The ide-checkout version is strictly more robust (survives a missing notification daemon on either OS) and never silently no-ops. **Better path:** when unifying, keep ide-checkout's cascade-with-final-fallback behavior, not cli-sessions' silent-discard.

### 10. Ghost-debug config-field schema duplicated

`ghost_opacity`/`ghost_debug_color` field blocks are byte-identical TOML between `plugins/plugin-launcher/qol-config.toml` (lines 15-28) and `plugins/plugin-alt-tab/qol-config.toml` (lines 124-137) - same `config_key` strings, same field definitions.

**Checked: `qol-config.toml` has no include/extends mechanism.** `libs/qol-config/src/lib.rs`'s loaders (`load_plugin_config`, `load_plugin_config_with_contract`, etc.) parse a single file each - there's no multi-file merge or `[import]`-style feature to hang a "shared schema" off of. Inventing one would be a real format-design change touching every plugin's config loading, not a small fix.

**Better path:** don't invent TOML includes for this. Add a cheap regression test (in whichever crate already tests config-schema validity) that reads both `qol-config.toml` files and asserts the `display_ghost_opacity`/`display_ghost_debug_color` blocks stay byte-identical. Matches this repo's existing convention of using a `--check`-style guard test rather than a new sharing mechanism (see how `qol-theme-css` enforces generated-CSS staleness).

### 11. Linux desktop-entry directory enumeration diverges

`libs/qol-app-icon/src/linux.rs:96,98` includes user-level `~/.local/share/flatpak/exports/share/applications` plus the system flatpak path. `libs/qol-apps/src/desktop.rs:121,124` includes `/usr/share/applications` plus the same system flatpak path but not the user-level one (at least not in the lines matched). Not a full side-by-side diff of every directory each checks.

### 12. `codesign.rs` platform-boundary violation, relocated but unaddressed

Moved wholesale from `apps/qol-tray/src/dev/build/cargo_build/codesign.rs` to `libs/qol-dev-build/src/cargo_build/codesign.rs` during the qol-dev-build extraction. Still exactly 13 `cfg(target_os = "macos")` splits, still no `platform/` facade - and `qol-dev-build` has no `platform/` directory at all. Arguably worth more attention now: this convention (strategy-pattern platform code, see `qol-project:qol-arch-code`) matters most for shared library crates that compile across the full CI platform matrix, and this file just moved from an app into exactly such a crate without being restructured.

Files with `cfg(target_os` outside any `platform/` directory, repo-wide, as of 2026-07-06 (**21 total**, not individually triaged for legitimacy - several are likely fine, e.g. `qol-platform/src/lib.rs` is probably the platform-detection entry point itself):

```
libs/qol-plugin-daemon/src/activation.rs
libs/qol-migrations/src/portability/paths.rs
libs/qol-dev-build/src/cargo_build/codesign.rs
libs/qol-runtime/src/broker/peer_cred.rs
libs/qol-app-icon/src/lib.rs
libs/qol-platform/src/lib.rs
libs/qol-gpui/src/popup_window/mod.rs
apps/qol-tray/src/main.rs
apps/qol-tray/src/doctor/checks/hotkey_shadows/mod.rs
apps/qol-tray/src/features/plugin_store/server/boot.rs
apps/qol-tray/src/features/plugin_store/server/logs_handlers.rs
apps/qol-tray/src/installer/mod.rs
plugins/plugin-window-actions/src/state_store.rs
plugins/plugin-launcher/examples/07_hide_show.rs
plugins/plugin-launcher/src/launch/mod.rs
plugins/qol-shot/src/region_selector.rs
plugins/plugin-keyremap/src/main.rs
plugins/plugin-pointz/src/input/mod.rs
plugins/plugin-alt-tab/src/preview_plane/backends/mod.rs
plugins/plugin-alt-tab/src/discovery/mod.rs
plugins/plugin-cli-sessions/src/notify.rs
```

### 13. Frecency silent error swallow (carried from 2026-07-01 backlog, still open)

`libs/qol-frecency/src/lib.rs:95` - `save()`'s `create_dir_all` error is silently dropped (`let _ = ...`) while the other 3 fallible calls in the same function `eprintln!` on error. Add the matching `eprintln!`.

### 14. Dead file (carried from 2026-07-01 backlog, still open)

`plugins/plugin-os-themes/src/cursor/platform/linux/x11_xfixes.rs` (19 lines) - not declared in the sibling `mod.rs`, zero references anywhere in the crate (reconfirmed with a fresh repo-wide grep this pass), not even compiled. Delete the file.

---

## Not reconfirmed this pass (uncertain - verify fresh before acting)

- **Unix-socket JSON-line framing "hand-rolled ~7x"**: now only 3 files match (`qol-runtime/src/client.rs`, `qol-runtime/src/watchdog.rs`, `qol-plugin-daemon/src/daemon.rs`) - likely substantially improved by an unrelated `client.rs::send()` consolidation from an earlier simplification pass, but not confirmed by diff.
- **Trace-log string-sanitizer "triplicated"** in plugin-launcher/plugin-cli-sessions: grep for `sanitize`/`strip_ansi`/`clean_log`/`scrub` found zero hits in either crate this pass - may have been renamed, removed, or never matched by these terms.
- **plugin-pointz daemon JSON response keys as string literals**: confirmed the pattern is still present (`daemon.rs:41-42`, raw `serde_json::json!({...})` with literal keys, no `qol-config` dependency), but not re-diffed against the config contract to confirm a "bypasses the guard" claim.
- **qol-shot config defaults defined twice / doctor bypass**: `cli.rs` still has `doctor_checks()` machinery; did not locate the second defaults definition this pass to confirm still-open.

---

## Suggested triage order (revised after the 2026-07-06 deep-dive; not a decision, just a starting point)

1. **Trivial, zero-risk**: #13 (frecency `eprintln!`), #14 (delete dead file). Both single-line/single-file changes already scoped since 2026-07-01.
2. **Confirmed dead code, remove per this repo's own CLAUDE.md rule**: #8 - `qol-plugin-daemon::focus` has zero callers repo-wide, reconfirmed by a fresh grep this pass. Simplest fix is deletion, not merging.
3. **Real, currently-reachable bugs, not just duplication**: #6's `ws.rs` half (untrusted network input silently defaulting instead of being rejected) and #3 (delete-key semantics - confirmed which side is wrong, but carries a real user-facing compatibility change, so scope the fix *with* a release note). #4 is plausibly connected to an already-shipped workaround for the same symptom - worth a git-blame check before fixing, high value if confirmed.
4. **Actively growing, worth stopping the bleeding**: #1 (DTO duplication - clean fix, client's struct is already a strict subset) and #2 (route strings - note that hoisting more constants alone already failed to get adopted once; pair with an enforcement mechanism, not just more constants).
5. **Everything else**: #5, #7 (now confirmed 3-platform in pointz alone), #9 (no security issue, just adopt ide-checkout's more robust fallback cascade), #10 (skip inventing TOML includes - a sync-guard test is enough), #11, #12 - genuine duplication/library candidates, lower urgency.
