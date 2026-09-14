# qol-memory notes run retention

Date: 2026-09-14
Branch: notes-retention
Status: implementation spec, architect-authored

## Problem

Each changed distill run writes a full corpus copy into a new timestamped
directory under `<store>/notes/<YYYY-MM-DDTHH-MM-SS-mmmZ>/` and nothing ever
removes old run directories. The live store currently holds 206 run directories
(511 MB) holding about 25 MB of unique content. Every product read path is
newest-only (`Store::read_notes` at src/store/mod.rs:101-118, `warm::newest_notes_run`
at src/app/warm.rs:359-369), so old run directories are dead weight.

Research tooling in docs/research/qol-memory does read historical runs
(`candidates.mjs:52-60` re-opens runs named by retrieval events; `verdict-eval.mjs`,
`test-alias.mjs`, `test-units-replace.mjs` pin `2026-08-13T16:31:40.844Z`). Until
that dependency is migrated to a frozen copy, automatic pruning must stay off by
default. This slice ships the retention capability with pruning disabled by
default, plus tests and a CLI lever. Activation on the live store is a later,
explicit step.

## Acceptance criteria

1. `Store::prune_notes_runs(keep)` removes whole run directories beyond the
   `keep` newest by run-name sort, never the newest run, never entries that
   fail `is_run_dir_name`, never follows symlinks, ignores per-directory
   removal failures, returns the count of directories actually removed, and
   returns `Ok(0)` for keep 0, missing root, or empty root.
2. A `notes_runs_kept` config knob (number, default 0, min 0, max 60, step 1)
   gates all automatic pruning. With 0, distill and warm behavior is
   byte-identical to today: no run directory is ever removed. With N greater
   than 0, pruning fires after a successful distill write and at daemon warm
   start.
3. A `prune` CLI subcommand acquires the distill lock, prunes with an explicit
   `--keep` or the config default, prints the removed count, supports `--store`,
   `--keep`, and `--json`, and works against `QOL_MEMORY_STORE`.
4. `DistillReport` and the notes `report.json` gain a `pruned` count. Prune
   events emit `qol_runtime::probe!` on the new `QOL_MEMORY_DISTILL` target
   with stable key=value fields and no absolute paths.
5. Tests cover the dense boundary matrix, the disabled-by-default regression,
   the lock-busy skip, non-run entry safety, and a deterministic invariant
   sweep (survivors equal the keep lexicographically greatest names). No new
   dependencies.
6. The architect verifies practically against a copy of the live store:
   206 to 7 run directories with the newest byte-identical, canary non-run
   entries intact, `status` and `doctor` still working on the copy, the live
   store untouched, and a default-config run removing nothing.

## Owned paths

One lane owns exactly these files, including their embedded test modules:

- plugins/memory/src/store/mod.rs
- plugins/memory/src/distill/mod.rs
- plugins/memory/src/app/mod.rs
- plugins/memory/src/app/warm.rs
- plugins/memory/src/watch/mod.rs
- plugins/memory/src/cli.rs
- plugins/memory/src/config/mod.rs
- plugins/memory/qol-config.toml

No other file may be edited. Cargo.toml is not owned because no dependency
changes are permitted.

## Design

### 1. src/store/mod.rs

Make `is_run_dir_name` and `newest_run_name` `pub(crate)` with unchanged logic.

Add to `impl Store`:

```rust
pub fn prune_notes_runs(&self, keep: usize) -> anyhow::Result<usize>
```

Exact semantics, in order:

1. `keep == 0` returns `Ok(0)` before touching the filesystem.
2. `read_dir` on `notes_root()`: `NotFound` returns `Ok(0)`; any other error
   returns `Err`.
3. Collect directory entry names passing `is_run_dir_name`. Sort ascending.
   The survivors are the last `keep` names; the removal set is the rest.
   If the collected count is at most `keep`, return `Ok(0)`.
4. For each name in the removal set, `remove_dir_all(notes_root().join(name))`.
   Count successes; ignore per-entry errors. Symlink entries named like runs
   fail `remove_dir_all` and survive uncounted, which is the required safety
   behavior: never follow a symlink out of the notes root.
5. Emit one probe (see trace contract) with the removed count and the survivor
   count, then return the removed count.

The method never acquires the distill lock itself; callers own locking.

### 2. src/distill/mod.rs

Change the signature:

```rust
pub fn run(store: &Store, notes_runs_kept: usize) -> Result<DistillReport>
```

Add `pub pruned: usize` to `DistillReport`. The unchanged early-return
constructs `pruned: 0`. After the successful rename of the new run directory
(distill/mod.rs:122), still inside the `_lock` scope, call
`store.prune_notes_runs(notes_runs_kept)`; on `Ok(removed)` use the count, on
`Err` print `qol-memory: notes prune failed: {error}` to stderr, emit the
error probe, and use 0. A prune failure must never fail the distill.

Add `"pruned": pruned` to the `stats` object of report.json.

Update every existing `run(` call site in this file's tests to
`run(&store, 0)` so current assertions hold unchanged.

### 3. src/app/mod.rs

Thread `config.notes_runs_kept` (loaded once at app/mod.rs:24) into the watch
spawn (app/mod.rs:36-46) and the initial-warm thread (app/mod.rs:47-64).
Neither watch nor warm may call `config::load()` itself. Adapt signatures
minimally; the value is `Copy`.

Add a private function:

```rust
fn prune_notes_runs_at_warm(store: &Store, notes_runs_kept: usize)
```

Semantics: keep 0 returns immediately. Otherwise
`DistillLock::acquire(store, "prune")`: `Ok(None)` means busy, emit the
lock-busy skip probe and return; `Ok(Some(_guard))` runs
`store.prune_notes_runs(notes_runs_kept)`, mapping `Err` to an stderr line
plus the error probe; `Err` from acquire maps to an stderr line plus the error
probe. Never panics, never propagates.

Call it in `run_initial_warm` immediately after the distill match block and
before `set_phase("building warm index")` (between app/mod.rs:131 and 132), so
`warm.layers()` and `newest_notes_run` observe the pruned set.

### 4. src/app/warm.rs

Delete the local `is_run_dir_name` copy (warm.rs:371-380) and the duplicated
newest-run listing inside `newest_notes_run` (warm.rs:359-369). Reimplement
`newest_notes_run` as a thin call to `crate::store::newest_run_name` over
`store.notes_root()`, preserving its current public shape and callers. No
third copy of the predicate may exist.

### 5. src/watch/mod.rs

Change the distill call at watch/mod.rs:63 to pass the threaded
`notes_runs_kept`. No other change.

### 6. src/cli.rs

New `prune` subcommand, modeled exactly on `distill`:

- A `USAGE_PRUNE` constant in the style of `USAGE_DISTILL` (cli.rs:27).
- A `PruneInvocation` with `store: Option<PathBuf>` and `keep: Option<u16>`,
  parsed by a token loop mirroring `parse_distill_invocation` (cli.rs:571-592):
  `--store` and `--keep` consume values via the `value_flag_with` helper;
  a non-numeric or out-of-range keep value and any unknown flag or positional
  produce a usage error string.
- Registration in `app_with_handlers` (cli.rs:82-106) with plain and JSON
  handlers, both routing through one `prune_report(invocation)` function
  mirroring `distill_report` (cli.rs:1014).
- `prune_report` resolves the store with `Store::resolve(invocation.store.as_deref())`,
  resolves keep as `invocation.keep.unwrap_or_else(|| config::load().notes_runs_kept)`
  cast to usize, acquires `DistillLock::acquire(&store, "prune")` where
  `Ok(None)` returns the busy error `qol-memory: prune busy`, and on
  `Ok(Some(_guard))` runs `store.prune_notes_runs(keep)?` and returns a
  `Serialize` report struct carrying `removed` and `keep`.
- Plain output line: `qol-memory prune: removed {removed} notes runs (keep {keep})`
  via `Execution::success(newline_terminated(...))`, mirroring cli.rs:996-1007.
- JSON output via `serde_json::to_value` of the report struct, mirroring
  cli.rs:1008-1012.

Change `distill_report` (cli.rs:1014-1018) to resolve
`config::load().notes_runs_kept as usize` and pass it to `distill::run`.
Extend the plain distill printer (cli.rs:996-1007) to include the pruned count
in its line. `config::load()` runs only inside the real registered handlers;
sentinel-based tests never execute it.

### 7. src/config/mod.rs and qol-config.toml

Add `pub notes_runs_kept: u16` to `Config`. Add to qol-config.toml:

```toml
[section.storage]
label = "Storage"
description = "How much disk the memory history keeps."

[field.notes_runs_kept]
type = "number"
variant = "slider"
label = "Notes run retention"
description = "How many recent notes run folders to keep on disk. 0 keeps everything and disables pruning. Takes effect after restarting Memory."
section = "storage"
default = 0
min = 0
max = 60
step = 1
```

The existing `verification_settings_match_the_config_contract` test then
validates the new default against the u16 field automatically.

## Trace contract

Target: `QOL_MEMORY_DISTILL`, uppercase and stable, joining the existing
`QOL_MEMORY_WATCH`, `QOL_MEMORY_DAEMON`, `QOL_MEMORY_INGEST` family. Invoke as
`qol_runtime::probe!("QOL_MEMORY_DISTILL", "...")` by full path with no import
(mirroring src/ingest/mod.rs:372). Message shape, stable key=value fields, no
prose, no absolute paths, no note content:

- success: `event=prune outcome=done removed={removed} kept={kept}` where kept
  is the surviving run directory count
- busy skip: `event=prune outcome=skip reason=lock_busy`
- failure: `event=prune outcome=error error={error}`

No probe when keep is 0: disabled means no events. Probe arguments must not
introduce locals that are unused in release builds; derive them from values
that already exist.

## Tests

All tests live in the owned files' existing test modules, following the
established `TempDir` helper pattern. No new dependencies; the invariant sweep
uses a deterministic in-test generator.

src/store/mod.rs:

1. Boundary table over a five-run fixture for keep in {0, 1, 4, 5, 6}: exact
   survivor name sets, exact removed counts, every removed directory gone,
   every survivor present with its notes.jsonl content intact.
2. Newest-run safety: a single run with keep 1 and keep 0 removes nothing.
3. Non-run entries survive: a loose file, a `.tmp-2026-09-14T01-00-00-000Z`
   directory, a `not-a-run` directory, malformed names (`2026-9-01T`,
   `notdigits`, `2026-09-01X00`), and a dangling symlink named like a run all
   remain after pruning, and the symlink is not followed.
4. Missing root and empty root return Ok(0).
5. Idempotency: an immediate second prune returns 0.
6. Deterministic invariant sweep: a small LCG implemented inside the test
   generates about 200 cases of canonical run-name sets (sizes 0 through 25)
   paired with keeps from {0, 1, 2, 5, 7, 10, 60}; for every case assert the
   survivors are exactly the keep lexicographically greatest names, the removed
   set is the rest, and the return value equals the removed count.

src/distill/mod.rs:

7. `run_prunes_old_runs_beyond_keep`: seed five old runs, run with keep 2; the
   new run is written, four old runs are removed, `report.pruned` is 4,
   report.json `stats.pruned` is 4, and `read_notes()` resolves the new run.
8. `run_with_keep_zero_removes_nothing`: seed five old runs, run with keep 0;
   all six run directories remain and `report.pruned` is 0.

src/app/mod.rs:

9. `prune_notes_runs_at_warm` with keep 0 leaves a seeded five-run pile
   untouched; with keep 2 it removes three; with the lock held via
   `DistillLock::acquire(store, "test")` it returns without pruning and
   without erroring.

src/cli.rs:

10. Prune parse tests mirroring the distill parse test (cli.rs:1441-1461):
    `--keep 7` parses to 7, `--store /tmp/x` parses, a missing value after
    `--keep` errors, a non-numeric keep errors, an unknown flag errors, and
    the sentinel handler receives the parsed invocation.

## Blast radius, architect-verified greps

The architect ran fixed-string greps on main at cfd7db5e5:

- `distill::run`: watch/mod.rs:63, cli.rs:1017, app/mod.rs:114, plus the
  unqualified `run(` calls inside src/distill/mod.rs tests. All owned.
- `is_run_dir_name`: store/mod.rs:124, store/mod.rs:143, app/warm.rs:365,
  app/warm.rs:371. All owned.
- `DistillReport`: distill/mod.rs:19, 28, 78, 123, cli.rs:1014. All owned.

No hit falls outside the owned set. `DistillReport` is Serialize-only with no
Deserialize anywhere, and every consumer of its fields reads only existing
keys, so the additive `pruned` field is safe (scout-verified: app/mod.rs:114-121
and watch/mod.rs:63-72 read `unchanged` only; cli.rs:985-1012 read or serialize
whole; distill tests assert existing fields only; report.json readers at
distill/mod.rs:317-327 and docs/research/qol-memory/decisions.mjs:413-420 are
additive-safe).

## Prohibitions for the lane

Edit only the owned paths. Never run build, test, lint, format, or git
commands. Add no code comments. Use no em-dash character anywhere. No new
dependencies. Report files and lines changed plus any conscious deviations,
and nothing else.

## Architect gate, not lane work

The architect runs, after the lane lands: `cargo fmt -- --check`,
`cargo clippy -p qol-memory --all-targets --all-features --keep-going -- -D warnings`,
`cargo test -p qol-memory --all-features`, and `cargo build -p qol-memory`,
then the practical live-store-copy verification with canaries, the
default-disabled proof, and the probe-line check in /tmp/qol-altmon.log.
