# qol-migrations

On-disk format migrations between qol-tray releases.

## Why this crate exists

When a qol-tray release changes the on-disk config layout (file paths, JSON
schemas, directory structure), users upgrading from the previous release have
data in the old layout. The daemon must either:

1. Refuse to start until something migrates the layout, or
2. Read both old and new layouts forever (interweaving migration code into
   every feature).

Option 2 is what most projects drift into. It rots the codebase: every feature
gradually accumulates "if old format do this, if new format do that" branches,
and removing the legacy branch becomes scary because callers everywhere depend
on it.

This crate enforces option 1, isolated from qol-tray's `src/`. Migrations live
here and only here. The daemon calls one function at boot; if there's nothing
to do, the call is free.

## Pattern: sliding-window release migrations

- Each release that breaks a contract ships exactly one migration file:
  `src/v3_15_to_v3_16.rs`.
- The active registry supports migrating from the previous **N** releases (3
  by default). Anything older: the daemon refuses to start and points the user
  at an intermediate release.
- Once a release falls outside the window, its migration file is deleted in a
  future release. The crate does not amass migration code forever.

## Trigger

- Auto at startup, dry-run + swap.
- The daemon calls `qol_migrations::run_if_needed(&config_dir)?` before any
  feature module reads config.
- Each migration runs in a temp dir, validates the result, then atomically
  swaps and archives the previous layout to
  `<config_dir>/archive/<migration-name>-<timestamp>/`.
- Failure: refuse to start, return the error chain. Per qol-tray non-negotiable
  #6 (failures are visible), the surfaced message must say what failed and
  where the archived copy lives.

## Adding a new migration

Default to **one folder per migration** so aux files (helpers, schema
converters, HTTP clients, split tests) have a home and pruning is a single
`rm -rf`. Layout:

```
src/
  lib.rs                       # trait + Registry + runner ONLY
  fs_util.rs                   # cross-migration helpers
  vN_to_vNplus1/
    mod.rs                     # Migration impl
    transforms.rs              # optional, when logic splits
    tests.rs                   # optional, when inline tests grow too long
fixtures/
  vN_to_vNplus1/before/        # snapshot of pre-migration config dir
  vN_to_vNplus1/after/         # expected post-migration state
```

Steps:

1. `mkdir src/vN_to_vNplus1/` and create `mod.rs` implementing the `Migration`
   trait.
2. Append the migration to `Registry::current()` in `src/lib.rs`.
3. Tests in `mod.rs` (inline) or `tests.rs` (split when inline grows past
   ~150 lines). Cover both `applies()` true / false cases and a full
   `migrate()` round trip asserting archive contents and resulting config-dir
   state.
4. If the new release pushes an old migration outside the support window,
   `rm -rf src/vN_to_vNplus1_oldest/ fixtures/vN_to_vNplus1_oldest/` in the
   same commit. Update `Registry::current()` to drop the entry. Bump the
   `qol-migrations` minor version.

## Why a separate crate, not a separate process

Migrations need to touch raw config files before the daemon can read them.
There is no daemon listening yet, so an HTTP-based microservice does not fit
the lifecycle. The crate is consumed two ways:

- As a library from the qol-tray daemon (one-line call at boot).
- As a standalone CLI via the `qol-tray-migrate` binary (in the qol-tray
  repo), for manual `--dry-run` debugging or running on a config dir other
  than the default.

Both paths share the same registry and the same migration implementations.
