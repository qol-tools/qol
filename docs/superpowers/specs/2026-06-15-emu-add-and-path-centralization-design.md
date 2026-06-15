# Add emulators from `qol dev`, on a centralized path convention

## Purpose

Today there is no first-class way to register a VM/emulator image with `qol`.
A user either drops a file into one of several scanned roots (auto-labeled
`x86_64`, which silently mislabels arm64 images) or hand-edits
`~/Library/Application Support/qol-tray/emu.toml`. There is no affordance in the
`qol dev` TUI or the `qol emu` CLI to add one.

This spec designs that affordance. While doing so it also fixes the substrate it
depends on: the qol-universe path convention (`config` / `data` / `cache` under a
`qol-tray` namespace) is consistent in concept but reimplemented inline in
several crates, so adding an emulator data dir naively would create yet another
copy. The work is therefore two parts, sequenced A before B:

- **Part A - centralize the path convention** so there is one source of truth.
- **Part B - add-emulator flow** in `qol dev` and `qol emu`, built on Part A.

## Design decisions (brainstorm outcome)

1. emu is a **dump** for any qemu-compatible image (Linux or Windows). Registration
   captures the one fact not in the file - **arch** - plus an optional **firmware**
   mode (default per arch, `uefi` for Windows). Not a bare open-folder, not a
   download catalog.
2. An image present in the designated dir but not yet registered is an
   **unconfirmed candidate** (`needs arch`); it is not runnable until confirmed.
3. v1 input is **open-folder + local registration only**. No URL download.
4. qol scans a **single designated emu dir**, with a derived default and a
   user override.
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

It is reimplemented inline rather than shared:

- [qol-config base_data_dir](/Users/kaho/repos/private/qol-monorepo/libs/qol-config/src/lib.rs:9)
  - `data_local_dir().or_else(data_dir).join("qol-tray")`, returns `Option`.
- [qol-tray base_data_dir](/Users/kaho/repos/private/qol-monorepo/apps/qol-tray/src/paths.rs:105)
  - byte-identical logic, returns `Result` with context.
- [qol-cli emu_config_path](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs:224)
  - `config_dir().join("qol-tray/emu.toml")` inline.
- [task_runner config](/Users/kaho/repos/private/qol-monorepo/apps/qol-tray/src/features/task_runner/config.rs:51)
  - `config_dir().join("qol-tray")` inline.

Notably, **no component uses an XDG state dir**; ephemeral data lives in
`/tmp/qol-tray` or `cache`.

### qol-config becomes the single source of truth

`qol-config` already owns the shared path surface that external consumers depend
on: [config_roots](/Users/kaho/repos/private/qol-monorepo/libs/qol-config/src/lib.rs:15)
(used by `qol-gpui`) and
[plugin_config_paths_from_env](/Users/kaho/repos/private/qol-monorepo/libs/qol-config/src/lib.rs:81)
(used by `plugin-os-themes`, `plugin-keyremap`, `plugin-lights`). Extend it with
the canonical base-dir API:

```rust
// libs/qol-config/src/lib.rs  (sketch)
pub const NAMESPACE: &str = "qol-tray";

pub fn config_dir() -> Option<PathBuf>; // dirs::config_dir()/qol-tray
pub fn data_dir()   -> Option<PathBuf>; // data_local_dir().or(data_dir)/qol-tray
pub fn cache_dir()  -> Option<PathBuf>; // dirs::cache_dir()/qol-tray

pub fn config_subdir(name: &str) -> Option<PathBuf>; // config_dir()/name
pub fn data_subdir(name: &str)   -> Option<PathBuf>; // data_dir()/name
pub fn cache_subdir(name: &str)  -> Option<PathBuf>; // cache_dir()/name
```

### Back-compat guarantees (review finding 1)

- `data_dir()` is a **non-breaking alias** of the existing `base_data_dir()`;
  `base_data_dir()` stays public and unchanged so nothing that imports it breaks.
- All new functions return `Option<PathBuf>`. `qol-config` stays `anyhow`-free;
  callers that want `Result` add their own context (see qol-tray below).
- `config_roots()` and `plugin_config_paths*()` are **not modified**. The install
  search order they produce stays byte-identical, so plugin config resolution and
  `qol-gpui` are unaffected.

### qol-tray delegates but keeps its test-root override (review finding 4)

qol-tray's [paths.rs](/Users/kaho/repos/private/qol-monorepo/apps/qol-tray/src/paths.rs:16)
override is **not** a simple global. It is a thread-local *stack* with nested
push/pop guards
([push_test_path_root](/Users/kaho/repos/private/qol-monorepo/apps/qol-tray/src/paths.rs:51)),
plus a second `dev`-gated env guard, plus the
`QOL_TRAY_TEST_PATH_ROOT` debug env which external integration tests rely on.

Decision: **wrap, do not move.** The override stays in qol-tray exactly as it is.
Only the production join is centralized:

```rust
// apps/qol-tray/src/paths.rs  (sketch)
pub(crate) fn base_data_dir() -> Result<PathBuf> {
    #[cfg(any(test, debug_assertions))]
    if let Some(root) = test_path_root() {
        return Ok(root.join("data").join(APP_NAME)); // override branch unchanged
    }
    qol_config::data_dir().context("Could not determine local data directory")
}
```

The thread-local stack, both guard types, the env var name, and their `cfg`
gating are untouched, so nested-guard test isolation and the documented
integration seam keep working. qol-tray re-exports nothing new.

### emu uses dependency injection, not the override

The emu subsystem already threads paths in rather than reaching for globals:
[DiscoveryContext](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/mod.rs:13)
carries `image_search_roots`, and discovery is pure over it. Part B keeps this:
the resolved emu dir and the `emu.toml` path are computed once at the command
boundary and passed into discovery/registry functions, so their tests use a
`TempDir` with no global override at all.

### task_runner and plugins

- `task_runner` switches its inline `config_dir().join("qol-tray")` to
  `qol_config::config_dir()`.
- `plugin-lights` already routes its primary path through
  `qol_config::plugin_config_paths_from_env`
  ([store.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-lights/src/config/store.rs:58));
  only its frozen legacy fallback
  ([store.rs](/Users/kaho/repos/private/qol-monorepo/plugins/plugin-lights/src/config/store.rs:88))
  is an inline `~/.config/qol-tray/...` path, and it stays frozen by design
  (review finding 5). The three config-driven plugins need no change.

### Rule doc first (Standards Evolution)

Before applying, encode the convention as a rule so future code conforms:
`qol-config` is the only place that knows the namespace and the `config` /
`data` / `cache` mapping; new components call `*_subdir`; there is no state dir.
Per the repo policy, the rule lands before the refactor that applies it.

## Part B - add-emulator flow

### Designated dir resolution

```
emu_dir = emu.toml top-level `dir` (with ~ expansion)  ||  qol_config::data_subdir("vms")
```

- macOS: `~/Library/Application Support/qol-tray/vms`
- Linux: `~/.local/share/qol-tray/vms`
- Windows: `%LOCALAPPDATA%\qol-tray\vms`

The `dir` key is a top-level string in `emu.toml`, parsed by
[discovery/config.rs](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/config.rs)
which already supports `~` expansion for image paths. In v1 `dir` is **user-set
only** (edited by hand or revealed via the open-folder button); we never rewrite
it programmatically, which avoids the TOML "top-level key before tables" hazard.
A missing dir is treated as empty and created on demand by the open action.

### Discovery change, and what "exclusive" means (review finding 2)

[discover()](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/mod.rs:21)
keeps combining three sources. The change is scoped to the **filesystem** source:

- Filesystem scan now looks in the **single** `emu_dir` only (replacing the old
  multi-root `image_search_roots`). This is the only thing that becomes
  exclusive.
- `[images.*]` config entries (which may point anywhere) and libvirt discovery
  are **unchanged** and still merged in.

Discovery returns both registered environments and unregistered candidates:

```rust
pub(crate) struct Discovered {
    pub(crate) environments: Vec<Environment>,   // registered, runnable
    pub(crate) candidates: Vec<ImageCandidate>,  // in emu_dir, not yet registered
}
```

A file in `emu_dir` is a candidate **iff** no `[images.*]` entry already
references it; once registered it is an `Environment`, not a candidate.

### The Unconfirmed model is a separate type (review finding 3)

[Environment.arch](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs:28)
is a mandatory `GuestArch`, and
[ResolveState](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs:38)
is `Ready | Missing | Unsupported`. An arch-less image therefore cannot be an
`Environment` without inventing an invalid state. So model it as its own type
rather than shoehorning a `ResolveState::Unconfirmed`:

```rust
pub(crate) enum Firmware { Bios, Uefi }

pub(crate) struct ImageCandidate {
    pub(crate) id: String,            // sanitized from filename, collision-suffixed
    pub(crate) path: PathBuf,         // file inside emu_dir
    pub(crate) display_name: String,
    pub(crate) arch: GuestArch,       // the one bit not in the file; inferred + toggleable
    pub(crate) firmware: Firmware,    // optional; default per arch, uefi for Windows
}
```

Registered `[images.*]` entries keep resolving to `Ready`/`Missing` exactly as
today. Scanning stays cheap: it lists files by extension via the existing
[is_vm_image_path](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu/discovery/mod.rs:11)
and infers arch by string match. It does **not** run `qemu-img` per file (that
would spawn a subprocess on every TUI poll); validation is deferred to confirm.

### Arch and firmware: the only required knobs

emu is a dump: any qemu-compatible image (Linux *or* Windows; `qcow2`/`raw`/
`vhdx`/`vmdk`/...) can be left in `emu_dir`. The one fact qol cannot read from the
file is **arch** - a disk image does not encode whether it is x86 or arm, and that
choice picks which `qemu-system-<arch>` binary launches (one cannot boot the
other's image). So arch is the single irreducible bit; everything else has a
default.

`arch = filename_heuristic(name).unwrap_or(host_native())`

- Heuristic: `arm64`/`aarch64` -> `Aarch64`; `amd64`/`x86_64`/`x64`/`i386`/`i686`
  -> `X86_64`; otherwise none.
- Host-native: map `std::env::consts::ARCH` (`aarch64` -> `Aarch64`, else
  `X86_64`).

This is right for the common case (an arm64 image on an arm Mac, or an
arch-named x86 download) and is correctable before confirm.

**Firmware** is the only other knob, and it is optional. `Uefi` is forced for
arm64 (as today) and is what Windows and UEFI-only Linux need; `Bios` is the x86
default. Inferred at confirm (arm64, or a `win`/`windows`/`vhdx` hint -> `Uefi`),
overridable via `--firmware` (CLI). The x86 resolve path gains OVMF
(`firmware_file()` returns the edk2/OVMF code blob for a `Uefi` x86 env instead of
`None`) so a `Uefi` choice actually boots - this is what lets Windows run, no
special-casing.

### Confirm = validate + write

Registration is a single pure-ish core, used by both surfaces:

```rust
fn register_image(emu_toml: &Path, dir: &Path, candidate: &ImageCandidate,
                  qemu_img: &Path) -> Result<String /* id */>;
```

Steps:
1. Run `qemu-img info --output=json <path>`; parse `format` and `virtual-size`.
   Reject (no write) if `qemu-img` is missing, errors, or the format is unknown.
2. Append a table to `emu.toml`, creating the file if absent and **preserving
   existing content** (text append, not round-trip, so hand edits and the `dir`
   key survive); skip if the id already exists:

   ```toml
   [images.<id>]
   path = "~/.local/share/qol-tray/vms/<file>"
   arch = "x86_64"
   firmware = "uefi"   # optional; omitted = arch default (bios on x86, uefi on arm)
   ```
3. Return the id; the caller triggers a rescan, after which the file resolves as
   a registered `Environment`.

### TUI surface (`qol dev`)

The emu list renders environments and candidates together; candidate rows show
`needs arch · <inferred arch>`. New context keys on the emu page (all currently
free in
[action_for](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/dev_console.rs:66)):

- `o` - open `emu_dir` in the OS file manager (created if missing). Desktop-only;
  on a headless host it instead prints the path.
- `t` - toggle the selected candidate's arch (`x86_64` <-> `aarch64`) in memory.
- `a` - confirm the selected candidate: `register_image` with the shown arch,
  then poke the emu poller to refresh.

Firmware uses its inferred default in the TUI; override it via
`qol emu add --firmware` (an in-TUI firmware toggle is deferred).

`a`/`t` act only on candidate rows. `qemu-img info` and the file write are a
one-shot on the keypress, not in any loop, so they do not affect idle cost.
Rendering follows the dev console's single-frame rules (no new full-page box).

### CLI surface (`qol emu`)

New verbs in the
[dispatch](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs:81):

- `qol emu add <path> [--arch x86_64|aarch64] [--firmware bios|uefi] [--id <id>]`
  - register an image. `--arch`/`--firmware` override inference. The path may be
  inside or outside `emu_dir`; outside paths are registered in place via an
  `[images.*]` entry.
- `qol emu open` - the CLI twin of the `o` action.

Both call the same `register_image` / open-folder cores as the TUI. This keeps
the feature usable headless and over SSH.

### Cross-platform open-folder

Add to `emu/platform`:

```rust
fn open_file_manager(dir: &Path) -> Result<()>; // open / xdg-open / explorer
```

It creates `dir` if missing before opening. When no file manager is available
(headless), the caller falls back to printing the path rather than erroring.

### Run state is unchanged

`run.log` / `report.json` stay under `target/qol-emu`
([last_runs_by_id](/Users/kaho/repos/private/qol-monorepo/tools/qol-cli/src/commands/emu.rs:125)).
No state dir is introduced, per decision 6.

## Testing

| Unit under test | Shape |
| --- | --- |
| `qol_config::*_subdir` + namespace | table test of resolver mapping |
| qol-tray override still nests | existing guard tests, unchanged |
| filename arch heuristic | table test incl. ambiguous/host-native fallback |
| id derivation + collision suffix | table test |
| `emu.toml` append (create, preserve, dup-skip) | integration test with `TempDir` |
| `qemu-img info` JSON parse | fixture-driven test (recorded JSON) |
| candidate vs environment partition | discovery test over a `TempDir` `emu_dir` |
| open-folder opener selection | unit test of per-OS argv (no spawn) |

Registry and discovery take their dir / `emu.toml` path as parameters, so all
I/O tests use a `TempDir` and need no global override.

## Implementation sequence

1. **A1** - rule doc for the path convention.
2. **A2** - `qol-config` API (`config_dir`/`data_dir`/`cache_dir` + `*_subdir`,
   `data_dir` alias), with tests.
3. **A3** - qol-tray `paths.rs` delegate (wrap), task_runner migrate; confirm the
   override tests still pass.
4. **B1** - emu dir resolution + single-dir filesystem scan + `dir` key parsing.
5. **B2** - `ImageCandidate` type + `Discovered` split in discovery.
6. **B3** - `register_image` core (qemu-img validate + toml append) + arch
   inference.
7. **B4** - `qol emu add` / `qol emu open` + `open_file_manager`.
8. **B5** - TUI `o` / `t` / `a` and candidate rendering.

Each step builds and tests green before the next.

## Out of scope (deferred)

- URL download / curated image catalog (the portable acquisition path).
- An in-TUI arch picker modal beyond the `t` toggle, and an in-TUI firmware toggle.
- A `qol emu dir set` writer for the top-level `dir` key.
- Promoting run state to an XDG state dir.
- Win11 install-time needs (TPM 2.0 / swtpm, Secure Boot), virtio driver
  provisioning, and serial automation (`run`/`sh`) for non-Debian guests. Plain
  Windows boot via `firmware = "uefi"` is **in** scope; these extras are not.

## Review findings folded in

1. qol-config compatibility - `data_dir` aliases `base_data_dir`, stays `Option`,
   `config_roots`/`plugin_config_paths*` byte-unchanged.
2. "Scanned exclusively" applies to the filesystem scan only; config and libvirt
   discovery stay active.
3. Unconfirmed modeled as a distinct `ImageCandidate`, not a `ResolveState`
   variant; registered entries remain runnable.
4. test-root override is wrapped, not moved; nested guards and
   `QOL_TRAY_TEST_PATH_ROOT` preserved; emu uses DI.
5. plugin-lights already uses `qol_config`; only its frozen legacy fallback stays
   inline.
