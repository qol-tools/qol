# Add emulators from `qol dev`, on a centralized path convention

## Purpose

Today there is no first-class way to register a VM/emulator image with `qol`.
A user either drops a file into one of several scanned roots (auto-labeled
`x86_64`, which silently mislabels an arm64 image) or hand-edits
`~/Library/Application Support/qol-tray/emu.toml`. There is no affordance in the
`qol dev` TUI or the `qol emu` CLI to add one.

This spec designs that affordance. While doing so it fixes the substrate it
depends on: the qol-universe path convention (`config` / `data` / `cache` under a
`qol-tray` namespace) is consistent in concept but reimplemented inline in
several crates, so adding an emulator data dir naively would create yet another
copy. The work is two parts, sequenced A before B:

- **Part A - centralize the path convention** so there is one source of truth.
- **Part B - add-emulator flow** in `qol dev` and `qol emu`, built on Part A.

This revision incorporates four review boards (2026-06-15). Round 1 (CONDITIONAL)
fixed the firmware chain, the task_runner claim, the legacy-root loss, the dedup
inventory, the new crate edge, and the TOML write contract. Round 2 (CONDITIONAL,
close) cleared the blocker and added: declare `toml_edit`, source the legacy
notice count, the `Environment.firmware` ripple, the merged-set exclusion basis,
the persistent (not "one-time") notice, the macOS log-dir literal, and the
B3/B4 firmware-parse ordering. Round 3 (CONDITIONAL, close) corrected the
`toml_edit` pin to `0.25` (lockfile-verified), folded `Environment.firmware` into
B3 so the widened parser tuple has a home, disposed of the orphaned
`DiscoveryContext.image_search_roots` field in B1, and reconciled the count helper
and owning constructor against the existing `dedupe.rs` walk. Round 4 (CONDITIONAL)
made the legacy-root advisory count only unregistered images, excluded logging
uniformly from the A3 migration (resolving a residual/migrate contradiction),
pinned the CLI `add` candidate-construction contract, and widened the A3 namespace
grep to catch slash-combined literals. See `## Review findings folded in`.

## Design decisions

1. emu is a **dump** for any qemu-compatible image (Linux or Windows).
   Registration captures the two facts not in the file: **arch** (required;
   picks the `qemu-system-*` binary) and **firmware** (`bios`/`uefi`, defaulted
   per arch, persisted). Not a bare open-folder, not a download catalog.
2. An image present in the designated dir but not yet registered is an
   **unconfirmed candidate** (`needs arch`); it is not runnable until confirmed.
3. v1 input is **open-folder + local registration only**. No URL download.
4. qol scans a **single designated emu dir** (derived default, user override).
   Dropping the old multi-root scan is a deliberate, user-visible change: a
   **persistent advisory** (not a silent removal) points users at the migration
   command, fed by a retained count-only legacy scan. See "Designated dir".
5. The default dir is **namespaced under `qol-tray` in the data dir**, matching
   the universe convention (not `~/VMs`, not a buried bespoke path).
6. Run state (`run.log`, `report.json`) is **not** promoted to an XDG state dir;
   the universe does not use one. It stays in `target/qol-emu`.

## Part A - centralize the path convention

### Current state

The convention every component already lands on:

| Bucket | Resolver | Namespace |
| --- | --- | --- |
| Config | `dirs::config_dir()` | `qol-tray/` |
| Data | `dirs::data_local_dir().or_else(dirs::data_dir)` | `qol-tray/` |
| Cache | `dirs::cache_dir()` | `qol-tray/` |
| Runtime (ephemeral) | hardcoded `/tmp/qol-tray` | `qol-tray/` |

Data/config derivations that hardcode the `qol-tray` namespace (the full
inventory, corrected and expanded across both review rounds):

- [qol-config base_data_dir](/Users/kaho/repos/private/qol-monorepo/libs/qol-config/src/lib.rs:9)
  - `data_local_dir().or_else(data_dir).join("qol-tray")`, `Option`.
- [qol-tray base_data_dir](/Users/kaho/repos/private/qol-monorepo/apps/qol-tray/src/paths.rs:105)
  - byte-identical, `Result`.
- [qol-tray legacy_config_dir](/Users/kaho/repos/private/qol-monorepo/apps/qol-tray/src/paths.rs:90)
  - `config_dir().join(APP_NAME)` inline.
- [qol-tray doctor/install_id](/Users/kaho/repos/private/qol-monorepo/apps/qol-tray/src/doctor/install_id.rs:22)
  - a 5th data-dir copy plus an `APP_NAME` literal.
- [qol-cli emu_config_path](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs:224)
  - `config_dir().join("qol-tray/emu.toml")` inline.
- [qol-cli dev active-worktree](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/dev.rs:230)
  - `config_dir().join("qol-tray/dev/active-worktree.txt")` inline.
- **Log dirs are a separate residual (review RR-DEDUP-MACOS + RR3 file_logger).**
  [logging/platform/macos.rs:5](/Users/kaho/repos/private/qol-monorepo/apps/qol-tray/src/logging/platform/macos.rs:5)
  is `home_dir().join("Library/Logs/qol-tray")`,
  [windows.rs:5](/Users/kaho/repos/private/qol-monorepo/apps/qol-tray/src/logging/platform/windows.rs:5)
  inlines `data_local_dir().join("qol-tray/logs")`, and
  [file_logger.rs:44](/Users/kaho/repos/private/qol-monorepo/apps/qol-tray/src/logging/file_logger.rs:44)
  has a `temp_dir().join("qol-tray/logs")` fallback. The macOS one **cannot**
  route through `qol_config::data_dir()` (macOS `data_dir` is `Application
  Support`, not `Library/Logs`; redirecting would relocate logs). All three are
  log-class literals that the A3 config/data grep excludes by class; they stay as
  acknowledged residuals, or move behind a future `qol_config` log-namespace
  helper (out of scope here).

`task_runner` is **not** in this list (review TR): its real path is
profile-scoped (see below). No component uses an XDG state dir.

### qol-config becomes the single source of truth

`qol-config` already owns the shared path surface external consumers depend on:
[config_roots](/Users/kaho/repos/private/qol-monorepo/libs/qol-config/src/lib.rs:15)
(used by `qol-gpui`) and
[plugin_config_paths_from_env](/Users/kaho/repos/private/qol-monorepo/libs/qol-config/src/lib.rs:81)
(three config plugins). Add only the functions with an in-scope caller (review
YAGNI dropped the speculative `cache_dir`/`cache_subdir`/`config_subdir`):

```rust
// libs/qol-config/src/lib.rs  (sketch)
pub const NAMESPACE: &str = "qol-tray";

pub fn data_dir()   -> Option<PathBuf>; // data_local_dir().or(data_dir)/qol-tray  (canonical)
pub fn config_dir() -> Option<PathBuf>; // dirs::config_dir()/qol-tray
pub fn data_subdir(name: &str) -> Option<PathBuf>; // data_dir()/name
```

### Back-compat and the two new dependency edges

- `data_dir()` is canonical; existing `base_data_dir()` becomes a thin
  `#[doc(hidden)]` alias of it (ALIAS follow-up: schedule converging the names).
  Both stay `Option`; keep `dirs::data_dir` fully qualified so the wrapper does
  not shadow confusingly.
- `config_roots()` and `plugin_config_paths*()` are **not** modified; the install
  search order stays byte-identical, so `qol-gpui` and the three config plugins
  are unaffected.
- **Edge 1 - qol-cli -> qol-config (review DEP).** qol-cli does not depend on
  qol-config today. `qol-config = { workspace = true }` is added to
  `tools/qol-cli/Cargo.toml` in **A3** (the first step where a qol-cli file,
  `dev.rs`, uses `qol_config`), not a pre-existing edge.
- **Edge 2 - toml_edit (review RR-TOML-EDIT + RR3-TOMLEDIT-VERSION).** The
  hardened TOML writer needs `toml_edit`, which is undeclared today (the workspace
  pins only `toml = "0.9"`, qol-cli only `toml.workspace = true`). Add `toml_edit`
  to root `[workspace.dependencies]` pinned **`0.25`** and to
  `tools/qol-cli/Cargo.toml`, sequenced in **B3**. The pin is lockfile-verified:
  `cargo add toml_edit -p qol` resolves to `0.25.12+spec-1.1.0`, which shares the
  `spec-1.1.0` substrate with the reader `toml 0.9.12+spec-1.1.0`; the `0.22.27`
  in the tree belongs to the unrelated `toml 0.8.23` (cbindgen/system-deps), not
  qol-cli's 0.9 reader, so `0.23` would not unify. `toml` and `toml_edit` coexist
  by role: `toml` (de)serializes/validates on read, `toml_edit` preserves
  formatting on write.

### qol-tray delegates but keeps its test-root override (review finding 4)

qol-tray's [paths.rs](/Users/kaho/repos/private/qol-monorepo/apps/qol-tray/src/paths.rs:16)
override is a thread-local *stack* with nested push/pop guards, a `dev`-gated env
guard, and the `QOL_TRAY_TEST_PATH_ROOT` debug env that external integration
tests rely on.

Decision: **wrap, do not move.** The override stays in qol-tray. Only the
production join is centralized:

```rust
// apps/qol-tray/src/paths.rs  (sketch)
pub(crate) fn base_data_dir() -> Result<PathBuf> {
    #[cfg(any(test, debug_assertions))]
    if let Some(root) = test_path_root() {
        return Ok(root.join("data").join(qol_config::NAMESPACE)); // override branch
    }
    qol_config::data_dir().context("Could not determine local data directory")
}
```

`legacy_config_dir()` delegates the same way to `qol_config::config_dir()`. The
thread-local stack, both guard types, the env var name, and their `cfg` gating
are untouched. The override branch references `qol_config::NAMESPACE` rather than
re-hardcoding it, so the now-redundant `APP_NAME` consts at
[paths.rs:7](/Users/kaho/repos/private/qol-monorepo/apps/qol-tray/src/paths.rs:7)
and [install_id.rs:7](/Users/kaho/repos/private/qol-monorepo/apps/qol-tray/src/doctor/install_id.rs:7)
are deleted in A3 (no-dead-code rule). Note `installer/source.rs:8`'s `APP_NAME`
is a binary-name argument, a different concern - it stays.

The other inline copies (`doctor/install_id`, `commands/dev.rs`, and qol-cli's
`emu_config_path`) migrate to `qol_config` in A3 so "single source of truth" holds
for config/data dirs. Logging is **not** partially migrated (review RR4-LOG-SCOPE):
the macOS, Windows, and temp-dir log literals are the acknowledged residuals above,
deferred together to a future log-namespace helper.

### emu uses dependency injection, not the override

[DiscoveryContext](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/mod.rs:13)
already carries paths in. The resolved emu dir and `emu.toml` path are computed
once at the command boundary
([run](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs:81))
and passed into discovery/registry functions, so tests use a `TempDir` with no
global override.

### task_runner is profile-scoped and stays put (review finding 2)

Its real path is
[task_runner_config_path()](/Users/kaho/repos/private/qol-monorepo/apps/qol-tray/src/features/task_runner/config.rs:46),
profile-scoped via the profile store. The inline
`dirs::config_dir().join("qol-tray")` at config.rs:50-55 is only
`fallback_config_path()`. A3 leaves the profile path untouched; it may optionally
delegate the *fallback* to `qol_config::config_dir()`. No behavioral change.

### plugin-lights (review finding 5)

Already routes its primary path through
[qol_config::plugin_config_paths_from_env](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-lights/src/config/store.rs:58);
only its frozen legacy `~/.config/qol-tray/...` fallback
([store.rs:88](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-lights/src/config/store.rs:88))
is inline, by design. No change.

### Rule doc first (Standards Evolution)

Encode the convention before applying it: `qol-config` is the single source for
**config and data dirs** (the namespace + the `config`/`data` mapping); new
components call `data_dir`/`config_dir`/`data_subdir`; there is no state dir. The
rule explicitly acknowledges the residual namespace literals it does **not**
cover: the qol-tray test-only override branch, and log-dir paths (macOS
`Library/Logs`, Windows `…/logs`) whose platform conventions differ from
`data_dir`.

## Part B - add-emulator flow

### Designated dir, and the legacy-root advisory (review SCAN + RR-NOTICE-N + RR-NOTICE-ONETIME)

```
emu_dir = emu.toml top-level `dir` (with ~ expansion)  ||  qol_config::data_subdir("emu")
```

- macOS: `~/Library/Application Support/qol-tray/emu`
- Linux: `~/.local/share/qol-tray/emu`
- Windows: `%LOCALAPPDATA%\qol-tray\emu`

The subdir token is `emu`, matching `emu.toml` / `qol-emu` / the `qol emu`
command. The `dir` key is a top-level string parsed by
[discovery/config.rs](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/config.rs);
in v1 it is user-set only. A missing dir is treated as empty, created on demand
by the open action.

**Migration is explicit, not silent.** Today's scan roots are real, populated
locations (macOS `~/VMs`, `~/Virtual Machines`, UTM; Linux also gnome-boxes and
`/media`,`/mnt` mounts; Windows `~/VMs`, `~/Virtual Machines`), enumerated by
`platform::image_search_roots` (three per-OS impls). The function is called once
([emu.rs:692](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs:692))
to fill [DiscoveryContext.image_search_roots](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/mod.rs:18),
whose sole consumer is
[filesystem::discover(&context.image_search_roots)](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/mod.rs:31).

**B1 disposition (review RR3-ORPHAN-CTX).** B1 repoints the filesystem scan at the
single `emu_dir`: `DiscoveryContext` carries an `emu_dir: PathBuf` **replacing**
the `image_search_roots` field, and mod.rs:31 scans that one dir. That would
orphan `platform::image_search_roots` and fail `-D warnings` (silently killing its
three per-OS impls), so its **sole remaining caller** becomes a count-only helper
`legacy_root_image_count(registered: &HashSet<PathBuf>) -> usize` that routes
through `platform::image_search_roots`, keeping the function and all three impls
reachable. It counts only **unregistered** legacy-root images: it excludes any
walked path whose canonical form is already in `registered` (the discovered
environments' canonical paths), so once a legacy-root image is registered the
advisory stops counting it (review RR4-LEGACY-STALE). The helper does **not** write
a second walk (review RR3-COUNT-DUP): the depth-limited `read_dir` +
`is_vm_image_path` filter + canonical de-dup at
[filesystem.rs:39/55](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/filesystem.rs:39)
is extracted into a path-collecting helper that both `filesystem::discover` and
`legacy_root_image_count` call; the count is the post-exclusion `paths.len()`.

It feeds a **persistent advisory** (not "one-time" - the empty state and doctor
are stateless re-renders, so no suppression marker is claimed) shown in the emu
empty state, `qol emu doctor`, and the `qol emu list` empty branch
([emu.rs:234](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs:234)):
`"N images found in legacy roots (~/VMs, ...); run `qol emu add <path>` to
register, or move them into <emu_dir>."` All three render sites are production
paths (not `cfg(test)`/headless-gated), so the `legacy_root_image_count` ->
`image_search_roots` keep-alive chain is live at runtime. `qol emu add` registers
an out-of-dir path in place via an `[images.*]` entry (no move); because the count
excludes registered canonical paths, registering in place clears the advisory for
that image just as moving it into `emu_dir` would.

### Discovery change, and what "exclusive" means (review finding 3 + PART + RR-DISCOVERED)

[discover()](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/mod.rs:21)
keeps combining sources. Only the **filesystem** source changes: it scans the
single `emu_dir` (replacing the multi-root primary scan). `[images.*]` config
entries (which may point anywhere) and libvirt discovery are unchanged.

Discovery returns registered environments and unregistered candidates:

```rust
pub(crate) struct Discovered {
    pub(crate) environments: Vec<Environment>,   // registered, runnable
    pub(crate) candidates: Vec<ImageCandidate>,  // in emu_dir, not yet registered
}
```

**One owning constructor** builds `Discovered`. It receives the **fully merged,
already-deduped registered set** (config + libvirt, run through the existing
[dedupe_and_sort](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/dedupe.rs:5),
which already canonicalizes and de-dups the merged set at dedupe.rs:16-20) plus
the raw `emu_dir` entries, and computes its **one new concern** - the
candidate/environment split - by excluding any `emu_dir` entry whose canonical
path matches a registered Environment's. It reuses the same `canonicalize` +
`is_vm_image_path` rule (filesystem.rs:39/55) as `dedupe_and_sort` rather than
standing up a second canonical-dedup (review RR3-CONSTRUCTOR-DUP); the merged-set
canonical invariant stays in `dedupe.rs`. So a file referenced by an `[images.*]`
entry **or a libvirt domain disk** whose path resolves into `emu_dir` appears once
(as the Environment), never double-listed. `is_vm_image_path` has a **second
consumer** - `teardown` at
[machine.rs:50](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/machine.rs:50)
(re-exported via mod.rs:11) - so the filesystem refactor keeps it exported and
signature-stable (review RR3-VMPATH-CONSUMER).

### Types: `ImageCandidate` and `Firmware` (review findings 3 + FW)

`Environment.arch` is a mandatory `GuestArch`
([emu.rs:29](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs:29))
and `ResolveState` is `Ready | Missing | Unsupported`, so an arch-less image
cannot be an `Environment`. Model the unconfirmed image as its own type:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]   // fieldless enum: Copy is free; Clone/Debug/Eq match Environment (which has no Copy)
pub(crate) enum Firmware { Bios, Uefi }         // as_str: "bios"/"uefi"; parse the inverse

pub(crate) struct ImageCandidate {
    pub(crate) id: String,            // sanitized from filename, collision-suffixed
    pub(crate) path: PathBuf,         // file inside emu_dir
    pub(crate) display_name: String,
    pub(crate) arch: GuestArch,       // inferred; toggleable before confirm
    pub(crate) arch_inferred: bool,   // true = from filename, false = defaulted to host
    pub(crate) firmware: Firmware,    // inferred per arch / OS hint
}
```

Registered `[images.*]` entries keep resolving to `Ready`/`Missing` as today.
Scanning stays cheap: it lists files by extension and infers arch/firmware by
string match; it does **not** run `qemu-img` per file (deferred to confirm).

### Arch and firmware inference (review findings FW + ARCH2)

emu is a dump: any qemu-compatible image can be left in `emu_dir`. The one fact
qol cannot read from a disk image is **arch** - it does not encode whether it is
x86 or arm, and that picks which `qemu-system-<arch>` binary launches. So arch is
the single irreducible bit.

```
arch = filename_heuristic(name).unwrap_or(host_native())   // arch_inferred = heuristic.is_some()
firmware = if arch == Aarch64 { Uefi }                      // arm: always UEFI (as today)
           else if win_hint(name) { Uefi } else { Bios }    // x86: BIOS default, UEFI for Windows
```

- Arch heuristic: `arm64`/`aarch64` -> `Aarch64`; `amd64`/`x86_64`/`x64`/`i386`/
  `i686` -> `X86_64`; else none.
- Host-native: map `std::env::consts::ARCH`.
- Windows hint: `win`/`windows`/`.vhdx` in the name.

**Scope of the fix (ARCH2).** This fixes the arm64 mislabel only for `emu_dir`
candidates. A candidate with `arch_inferred == false` renders `needs arch · <arch>
(host default)` so the user knows `t` matters; arch-less `[images.*]` entries keep
the legacy `X86_64` default (`qemu-img` cannot recover arch from a disk).

### Confirm = validate + write (review finding TOML + RR-REGISTER-REUSE)

```rust
fn register_image(emu_toml: &Path, candidate: &ImageCandidate,
                  qemu_img: &Path) -> Result<String /* id */>;
```

1. Run `qemu-img info --output=json <path>`; parse `format` and `virtual-size`.
   Reject (no write) if `qemu-img` is missing, errors, or the format is unknown.
2. Write the `[images.<id>]` table with `toml_edit` (declared per Edge 2):
   - **Read + parse first** to test membership, reusing
     [parse_image_overrides](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/config.rs:39)
     as the dedup oracle (promoted to `pub(crate)`). Its stored paths are
     `expand_home`-only (config.rs:66) while discovery excludes by `canonicalize`
     (filesystem.rs:39), so `register_image` **canonicalizes both the candidate
     path and the map values** before comparing (review RR3-CANON-BASIS); id-dedup
     then shares discovery's canonical basis and a symlinked/relative `[images.*]`
     path cannot dodge a dup. It also catches a pre-existing malformed file; on
     parse failure **fail the add with a clear error**, do not append onto
     unparseable content.
   - Skip if the id already exists.
   - `toml_edit` preserves the user's `dir` key and comments and places the new
     table correctly, sidestepping the top-level-key-before-tables and
     trailing-newline hazards of raw append; it also quotes values (Windows
     `vhdx` backslash paths).
   - Concurrency stance: single-user best-effort (the dev console is one
     process); concurrent `add`/`a` is out of scope.

   ```toml
   [images.<id>]
   path = "~/.local/share/qol-tray/emu/<file>"
   arch = "x86_64"
   firmware = "uefi"   # omitted = arch default (bios on x86, uefi on arm)
   ```
3. Return the id; the caller pokes a rescan, after which the file resolves as a
   registered `Environment`.

### Firmware resolve chain (review finding FW + RR-ENV-FIRMWARE + RR-FW-CALLER)

The must-fix that makes Windows boot without regressing x86:

1. **Persist + parse.** Extend `parse_image_overrides`
   ([config.rs:49](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/config.rs:49))
   to read an optional `firmware` key, defaulting per arch (`x86 -> Bios`,
   `arm -> Uefi`). The override map carries `(PathBuf, GuestArch, Firmware)`.
2. **Carry it through every `Environment` construction site.** Add
   `firmware: Firmware` to `Environment`
   ([emu.rs:29](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs:29));
   there is no `Default`, so update all five literals explicitly:
   - [config.rs:12](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/config.rs:12)
     - the parsed firmware.
   - [libvirt.rs:19](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/libvirt.rs:19)
     - arch-derived default (libvirt has no firmware concept; its hardcoded
     `X86_64` -> `Bios`).
   - [filesystem.rs:44](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/filesystem.rs:44)
     - n/a once the filesystem source emits `ImageCandidate`s, but if any
     `Environment` remains here it carries the inferred firmware.
   - the two test literals (emu.rs:1059, emu.rs:1099) - any valid value.
   Also serialize `firmware` in the hand-written
   [report_json](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs:920)
   (it lists fields explicitly, so the field is dropped silently otherwise), with
   a round-trip assertion.
3. **Select on `(arch, firmware)`.** Replace
   [firmware_file(self)](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/arch.rs:39)
   with a function of both:
   - `(X86_64, Bios) -> None`  - **unchanged**; x86 BIOS never gated on a blob.
   - `(X86_64, Uefi) -> ["edk2-x86_64-code.fd", "OVMF_CODE.fd", "OVMF_CODE_4M.fd"]`
   - `(Aarch64, _)   -> ["edk2-aarch64-code.fd"]`  - arm is always UEFI.
4. **Locate with multi-candidate search.**
   [locate_firmware](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs:761)
   returns `Ok(None)` for the no-blob case (existing x86 stays un-gated, no
   regression), else searches the candidate filenames across `<bin>/../share/qemu`
   plus fallbacks (`/usr/share/qemu`, `/usr/share/OVMF`, `/usr/share/edk2/x64`).
   First hit wins; a `Uefi` env with none found -> `Err` -> `Unsupported` with a
   reason naming the candidates (reached only for explicit UEFI). Update the
   **production caller** at
   [emu.rs:723](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs:723)
   to pass `environment.firmware`. For q35 + OVMF, wire the blob via `pflash` (as
   the aarch64 `virt` path already does), not legacy `-bios`.
5. **Update the test.** Keep
   [locate_firmware_finds_edk2_next_to_binary](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs:1146)
   asserting `(X86_64, Bios) == Ok(None)`; **add** an `(X86_64, Uefi)` missing ->
   `Err` and found -> `Ok(Some)` case. Do not mutate the x86-default assertion.

The captured firmware mode survives registration (parsed back), threads through
`Environment` and `report.json`, and drives selection.

### TUI surface (`qol dev`) (review finding KEY)

The emu list renders environments and candidates together; candidate rows show
`needs arch · <arch>` (with `(host default)` when `arch_inferred == false`). New
keys, free in
[action_for](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/dev_console.rs:66):

- `o` - open `emu_dir` in the OS file manager (created if missing); on a headless
  host it prints the path instead.
- `t` - toggle the selected candidate's arch (`x86_64` <-> `aarch64`) in memory.
- `a` - confirm the selected candidate: `register_image` with the shown arch,
  then poke the emu poller to refresh.

`action_for` is global, so the emu-page gate lives in `apply_action` under the
existing `match dash.view` arms; `o`/`t`/`a` are no-ops off the emu page, and
`t`/`a` are no-ops off a candidate row. Firmware uses its inferred default in the
TUI; override it via `qol emu add --firmware` (an in-TUI firmware toggle is
deferred). Rendering follows the dev console single-frame rules.

### CLI surface (`qol emu`)

New verbs in the dispatch `match command` inside
[run()](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs:81):

- `qol emu add <path> [--arch x86_64|aarch64] [--firmware bios|uefi] [--id <id>]`
  - register an image; `--arch`/`--firmware` override inference. The path may be
  inside or outside `emu_dir`; outside paths register in place via `[images.*]`.
- `qol emu open` - the CLI twin of `o`.

Both call the same `register_image` / open cores as the TUI, so the feature works
headless and over SSH. The CLI builds the `ImageCandidate` that `register_image`
expects from `<path>` (review RR4-ADD-CONTRACT): the same filename inference used
for discovery candidates fills `arch`/`firmware`/`display_name`/`id`, then `--arch`,
`--firmware`, and `--id` override those fields. `--id` is run through the same
`sanitize_id` + collision-suffix rule as a filename-derived id (sanitized, never
rejected), so there is one id-construction path and one `register_image` contract
across both surfaces; no separate request type is introduced.

### Open-folder reuses the existing platform op (review finding OPEN)

qol-cli already has
[trait PlatformOps](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/platform/mod.rs:23)
with per-OS `open_url`. Add a sibling `open_path(&self, dir: &Path)` method (or
reuse the `open` crate as qol-tray does), keeping the dir-create-if-missing
wrapper at the emu boundary - not a bespoke parallel opener.

### Run state is unchanged

`run.log` / `report.json` stay under `target/qol-emu`
([last_runs_by_id](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs:125)).

## Testing (review finding DOD)

| Unit under test | Shape |
| --- | --- |
| `qol_config::{data_dir,config_dir,data_subdir}` + `NAMESPACE` | table test of resolver mapping |
| qol-tray override still nests | existing guard tests, unchanged |
| filename arch heuristic + `arch_inferred` flag | table incl. host-default fallback |
| firmware inference (arch + win hint) | table test |
| `firmware_file(arch, firmware)` selection | table incl. `(x86,bios)->None`, `(x86,uefi)->candidates` |
| `locate_firmware` multi-candidate | tempdir: x86-uefi missing->Err, found->Ok(Some), x86-bios->Ok(None) |
| `[images.*]` firmware round-trip | parse a written `firmware=uefi` back (after B3 parser extension) |
| `report.json` carries firmware | serialize an env, assert `firmware` present |
| id derivation + collision suffix | table test |
| `register_image` via `toml_edit` | tempdir: create, preserve `dir`/comments, dup-skip, malformed-fail |
| `qemu-img info` JSON parse | fixture-driven |
| `Discovered` partition incl. `[images.*]`- and libvirt-into-`emu_dir` overlap | tempdir; assert single-listing |
| `legacy_root_image_count` + advisory | tempdir with images in legacy roots: assert **non-zero N**; then pass one as registered and assert it drops from the count (RR4-LEGACY-STALE) |
| `open_path` opener selection | unit test of per-OS argv (no spawn) |

Registry/discovery take dir / `emu.toml` path as parameters; all I/O tests use a
`TempDir`. End-to-end DoD: confirm an arm64-named image, assert `arch="aarch64"`
in `emu.toml` and that it resolves `Ready` on an arm host; the candidate row
showed `needs arch · aarch64` and `t` flipped it before `a`.

## Implementation sequence

1. **A1** - rule doc for the path convention (config/data only; log-dir residuals
   acknowledged).
2. **A2** - `qol-config` API (`data_dir` canonical, `config_dir`, `data_subdir`,
   `base_data_dir` alias, `NAMESPACE`), with tests. *DoD: resolver table test
   green; `base_data_dir` returns identical paths to before.*
3. **A3** - add `qol-config = { workspace = true }` to `tools/qol-cli/Cargo.toml`;
   migrate `base_data_dir`, `legacy_config_dir`, `doctor/install_id`, qol-cli
   `dev.rs`, and qol-cli `emu_config_path` (emu.rs:224) to qol-config; delete the
   now-dead `APP_NAME` consts at `paths.rs:7` and `install_id.rs:7`; keep the
   override wrapping. Logging is **not** migrated (uniform residual). *DoD: override
   tests still pass; the namespace grep `rg 'join\("qol-tray|join\(APP_NAME'` -
   unanchored `join("qol-tray` so it catches slash-combined literals like
   `join("qol-tray/emu.toml")` (review RR4-GREP-NARROW) - finds only the two
   residual `join("qol-tray/logs")` literals (windows.rs:5, file_logger.rs:44)
   outside qol-config; the override branch uses `join(NAMESPACE)` and macOS uses
   `join("Library/Logs/...")`, neither matching.*
4. **B1** - emu dir resolution + single-`emu_dir` filesystem scan + `dir` parsing;
   retain `image_search_roots` behind `legacy_root_image_count()`; wire the
   advisory into empty state, doctor, and `qol emu list` empty branch. *DoD:
   `qol emu list` reads `emu_dir`; with images in legacy roots the advisory shows
   non-zero N.*
5. **B2** - `ImageCandidate`/`Firmware` types + `Discovered` owning constructor
   over the merged registered set. *DoD: an `emu_dir` file shows as a candidate;
   an `[images.*]`/libvirt file pointing into `emu_dir` does not double-list.*
6. **B3** - `qemu-img` validate + `toml_edit` write (declare `toml_edit = "0.25"`
   dep); promote `parse_image_overrides` to `pub(crate)` and widen it to the
   `(PathBuf, GuestArch, Firmware)` tuple reading `firmware`; arch/firmware
   inference. **Fold `Environment.firmware` (+ all five literals + `report.json`
   serialization) into this step** (review RR3-B3-UNUSED-FW) so the widened tuple
   has a storage home the moment it lands: the `discover` consumer at
   config.rs:10-12 would otherwise hold an unused `firmware` binding and fail
   `-D warnings`. *DoD: confirming an arm64-named image writes `arch="aarch64"`; a
   written `firmware="uefi"` round-trips through the parser into
   `Environment.firmware`; `report.json` carries `firmware`; a malformed
   `emu.toml` fails the add.*
7. **B4** - resolve-side firmware *selection* (the field already exists from B3):
   `firmware_file(arch, fw)`, `locate_firmware` multi-candidate, and widening the
   single-arg `locate_firmware(path, arch)` call (emu.rs:723 production, emu.rs:1155
   test) to `(path, arch, firmware)` fed `environment.firmware`. *DoD: x86-bios
   still `Ok(None)`; x86-uefi with OVMF present resolves `Ready`, absent
   `Unsupported` with reason.*
8. **B5** - `qol emu add`/`open` + `PlatformOps::open_path`; TUI `o`/`t`/`a` with
   the emu-page gate and candidate rendering. *DoD: `o` opens `emu_dir`; `a`
   registers the selected candidate and it flips to `Ready` on refresh.*

Each step builds and tests green before the next.

## Out of scope (deferred)

- URL download / curated image catalog.
- An in-TUI arch picker modal beyond `t`, and an in-TUI firmware toggle.
- A `qol emu dir set` writer for the top-level `dir` key.
- Promoting run state to an XDG state dir.
- Win11 install-time needs (TPM 2.0 / swtpm, Secure Boot), virtio driver
  provisioning, and serial automation (`run`/`sh`) for non-Debian guests. Plain
  Windows boot via `firmware = "uefi"` + OVMF is **in** scope; these are not.
- **Cross-version (review RR-XVERSION).** An older `qol` binary lacks the
  `firmware` parser, so a written `firmware = "uefi"` is ignored and an x86
  Windows image silently boots BIOS (fails). This is forward-compat-by-ignore,
  acceptable for a single-user tool but recorded so a post-rollback failure has a
  documented cause.
- **ALIAS** - converging `data_dir`/`base_data_dir` to one name.
- **DOCS** - updating `qol-cli-commands` SKILL.md key grammar and the
  `emu.rs:449/969` hardcoded `~/.config/qol-tray/emu.toml` hint strings, and a
  future `qol_config` log-namespace helper for the macOS/Windows log dirs.

## Open questions (resolved by this revision)

1. Notice also on the `qol emu list` empty path? **Yes** - wired into
   `cmd_list`'s empty branch (emu.rs:234), not just the TUI/doctor.
2. `toml_edit` version? Pin `0.25` (conventional caret form of the
   lockfile-resolved `0.25.12+spec-1.1.0`, the editing crate paired with
   `toml 0.9`), declared deliberately rather than drifting to the unrelated
   `0.22`/`0.8` tree.
3. Full enumeration or count-only legacy scan? **Count-only**
   (`legacy_root_image_count`), cheaper and avoids throwaway `Environment`s.
4. Is the advisory render path reachable at runtime (not `cfg(test)`/headless)?
   **Yes** - the empty state, `qol emu doctor`, and the `qol emu list` empty branch
   (emu.rs:234) are production render paths, so the `legacy_root_image_count` ->
   `image_search_roots` keep-alive is live (review RR3 open question).

## Review findings folded in

Round 1: FW (firmware chain), TR (task_runner profile-scoped), SCAN (explicit
migration), DEDUP (expanded inventory), DEP (A3 edge), TOML (`toml_edit` design),
PART (owning constructor), YAGNI (trimmed API), NAME (`emu` token), OPEN
(`PlatformOps`), ARCH2/KEY/DOD.

Round 2:

| id | resolution |
| --- | --- |
| RR-TOML-EDIT (high) | `toml_edit` declared in root workspace + qol-cli, pinned `0.25` (see RR3-TOMLEDIT-VERSION), sequenced in B3 (Edge 2). |
| RR-NOTICE-N (high) | `image_search_roots` retained behind a count-only `legacy_root_image_count()` feeding the advisory; kept in inventory; test asserts non-zero N. |
| RR-ENV-FIRMWARE (med) | All five `Environment` literals enumerated with the firmware each assigns; `report.json` serializes `firmware` with a round-trip test; `Firmware` derives match `Environment`. |
| RR-DISCOVERED (med) | Exclusion basis is the merged config + libvirt registered set, not just `[images.*]`. |
| RR-NOTICE-ONETIME (med) | "one-time" dropped; it is a persistent advisory (no suppression marker claimed). |
| RR-DEDUP-MACOS (med) | macOS/Windows log-dir literals added to inventory as acknowledged residuals (macOS can't route through `data_dir`); rule-doc claim scoped to config/data dirs; A3 DoD broadened to a namespace-literal grep. |
| RR-DEP-SEQ (med) | DEP edge is A3 in body, table, and sequence (consistent). |
| RR-REGISTER-REUSE (med) | `parse_image_overrides` promoted to `pub(crate)`; firmware-parse extension folded into B3 so the round-trip is asserted there, not before the parser carries `Firmware`. |
| RR-FW-CALLER (low) | `locate_firmware` production caller at emu.rs:723 updated in B4. |
| RR-XVERSION (low) | Old-binary silent BIOS downgrade documented in Out of scope. |
| RR-DEADCONST (low) | Redundant `APP_NAME` consts (paths.rs:7, install_id.rs:7) deleted in A3; `installer/source.rs:8` (binary-name arg) stays. |

Round 3:

| id | resolution |
| --- | --- |
| RR3-TOMLEDIT-VERSION (high) | Pin corrected `0.23+` -> `0.25` in Edge 2, Open-question 2, and the Round-2 table; lockfile-verified via `cargo add toml_edit -p qol` (`0.25.12+spec-1.1.0`, shares the `spec-1.1.0` substrate with `toml 0.9.12`). |
| RR3-B3-UNUSED-FW (high) | `Environment.firmware` (+ five literals + `report.json`) folded into B3 so the widened `(PathBuf, GuestArch, Firmware)` parser tuple has a storage home at the boundary; B4 is now selection-only. |
| RR3-ORPHAN-CTX (high) | B1 replaces `DiscoveryContext.image_search_roots` with `emu_dir: PathBuf`; `platform::image_search_roots`'s sole remaining caller is `legacy_root_image_count`, keeping all three per-OS impls reachable; the "one site" claim is corrected to the function-call site plus its field consumer at mod.rs:31. |
| RR3-COUNT-DUP (med) | `legacy_root_image_count` reuses an extracted path-collecting walk (`read_dir` + `is_vm_image_path` + canonical de-dup at filesystem.rs:39/55), returning `paths.len()`; no parallel counter. |
| RR3-CONSTRUCTOR-DUP (med) | Owning constructor consumes the already-deduped merged set from `dedupe_and_sort` (dedupe.rs:5) and only owns the candidate/environment split; the merged-set canonical invariant stays in `dedupe.rs`. |
| RR3-VMPATH-CONSUMER (med) | Noted `is_vm_image_path`'s second consumer `teardown` (machine.rs:50, re-export mod.rs:11); the refactor keeps it exported and signature-stable. |
| RR3-CANON-BASIS (med) | `register_image` canonicalizes both the candidate path and the `parse_image_overrides` map values (`expand_home`-only at config.rs:66) so id-dedup shares discovery's canonical basis. |
| RR3 anchors / file_logger (low/note) | dev.rs literal `:227`->`:230`; DOCS hint `:968`->`:969`; `file_logger.rs:44` temp-dir log fallback added to the log-residual inventory; advisory render-path reachability confirmed (Open-question 4); `Firmware` `Copy` comment clarified (Environment has no `Copy`). |

Round 4:

| id | resolution |
| --- | --- |
| RR4-LEGACY-STALE (med) | `legacy_root_image_count(registered: &HashSet<PathBuf>)` counts only legacy-root images whose canonical path is **not** already registered, so a register-in-place clears the advisory just like a move; test asserts the count drops after registration. |
| RR4-LOG-SCOPE (med) | Self-contradiction (windows-log listed as both residual and A3-migrated) resolved by excluding logging uniformly from A3: `logging/platform/windows` dropped from the migrate list; macOS + Windows + temp-dir log literals are residual, deferred together to a future log-namespace helper. |
| RR4-ADD-CONTRACT (low) | CLI `add` builds the `ImageCandidate` via the discovery filename inference, then applies `--arch`/`--firmware`/`--id`; `--id` is `sanitize_id`-sanitized (never rejected). One id path, one `register_image` contract, no new request type. |
| RR4-GREP-NARROW (low) | A3 DoD grep widened to `rg 'join\("qol-tray|join\(APP_NAME'` (unanchored prefix catches `join("qol-tray/emu.toml")`); `emu_config_path` (emu.rs:224) added to the A3 migration so no config-dir namespace literal is left; allowlist = the two `join("qol-tray/logs")` residuals. |
