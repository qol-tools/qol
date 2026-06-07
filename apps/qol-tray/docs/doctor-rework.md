# qol-tray doctor — composable registry + always-on health

Status: **locked spec; implementation starting at Stage 1.** Design settled over a
research pass on Flutter, React Native, Expo, npm, and Homebrew doctors. This is
the build contract; the Decision Log records *why* so choices are not re-litigated.

## Design invariant (the spine)

**Complex in the engine, simple in the interface** (your `CLAUDE.md` deep-module
rule / Ousterhout). Two interfaces must stay trivial while the engine absorbs the
complexity:

- **Check-author interface:** a check *reports findings*; the engine *decides the
  verdict* (status rollup, fix-availability, duration, crash). Authors never set
  rolled-up state.
- **User/dev interface:** **silent when healthy** (the single-digit-second boot
  stays clean), **one quiet line when auto-fixed**, **one unmissable, actionable
  banner when unfixable**. Nothing else.

The doctor is never a tool you remember to invoke. It runs as a pre-launch gate
and inside the dev loop, and only speaks when it has something you must act on.

## Goal / non-goals

Goal: curated check registry, every check individually addressable, structured
`issues` + `advice` + `duration`, crash-isolated runs, mission-correct fix
authority, `fingerprint_health` as the first native check, and always-on
execution + visibility so drift surfaces itself.

Non-goals (deferred; nothing blocks them): third-party doctor extensions,
parallel execution, generic per-check timeout harness, an `/api/doctor` route,
`FixCost`/`RebuildPlugin` (see Deferred).

## Current state (refs)

- Pre-launch gate **already exists**: `main.rs` runs the startup doctor before the
  tray comes up; recompile re-execs the binary so it re-runs every dev cycle.
- Registry `doctor/checks.rs` — hardcoded `Vec`; `collect_diagnosis` matches only
  2 of 8 checks (`CheckId` is the bottleneck).
- Model `doctor/diagnosis.rs` (`Diagnosis`, central `FixAction` + `apply_fix` +
  `is_safe_to_auto_apply`), `doctor/report.rs`.
- Dev-loop coverage is **partial**: recompile runs only `PluginProcessLeaks`
  (`restart_schedule.rs`); dev-drift checks never run there.
- **Suppressed**: `logging/control.rs` suppress-patterns + `suppressed-errors.json`
  mute doctor findings. A banner surface exists (`ui/components/BootHealedBanner.js`)
  but is wired to a *single* finding (`boot_target` drift), not doctor results.
- Consumers outside `doctor/`: only `restart_schedule.rs`. No HTTP route, no UI render.
- `trigger.rs` already keys by `check_id: String` — single-run is string-native.

## Target types (`doctor/framework.rs`)

```rust
pub trait DoctorCheck {                       // no Send+Sync: runner is synchronous
    fn meta(&self) -> CheckMeta;
    fn run(&self, ctx: &DoctorContext) -> CheckReport;     // EMIT-ONLY
}

pub struct CheckReport {                       // what the author returns — small
    pub summary: String,                       // the "all good" / headline line
    pub issues: Vec<DoctorIssue>,
    pub advice: Vec<String>,
    pub fixes:  Vec<FixAction>,
}
impl CheckReport {
    pub fn ok(summary: impl Into<String>) -> Self;         // healthy one-liner
    // problems: push issues / advice / fixes
}

pub struct DoctorIssue {
    pub code: &'static str,
    pub severity: Severity,                    // the ONLY status enum an author touches
    pub message: String,
    pub evidence: Vec<String>,
}
pub enum Severity { Info, Warn, Error, Crash } // Crash = the check itself failed to run
```

`CheckMeta` has defaults so the common case is `id` + `label`; the engine fills
the machinery:

```rust
pub struct CheckMeta {
    pub id: &'static str,
    pub label: &'static str,
    pub category: CheckCategory,
    pub groups: &'static [&'static str],       // cross-cutting selection tags
    pub platform: PlatformScope,
    pub dev_only: bool,
    pub order: u16,                            // DISPLAY/EXEC order only; default 0 → registry index breaks ties
}
impl CheckMeta {
    pub const fn new(id, label, category) -> Self;         // platform=Any, groups=&[], dev_only=false, order=0
    pub const fn group(self, &'static [&'static str]) -> Self;
    pub const fn platform(self, PlatformScope) -> Self;
    pub const fn dev_only(self) -> Self;
}
pub enum CheckCategory { Install, HostSurface, Plugins, Runtime, DevBuild }
pub enum PlatformScope { Any, Linux, Macos, Windows }

pub struct DoctorContext {                     // load each source ONCE, one snapshot
    config_dir: PathBuf,
    registry:     OnceCell<Result<Registry, String>>,
    fingerprints: OnceCell<BuildFingerprints>,
    linked:       OnceCell<Vec<LinkedPlugin>>,
}                                              // memoized accessors kill 3x load_registry()

pub enum Selector { All, Id(String), Group(String), Category(CheckCategory) }
```

The **engine** derives the per-check record consumers see — the author never sets it:

```rust
pub struct DoctorCheckResult {
    pub outcome: Outcome,        // DERIVED, not author-set
    pub issues: Vec<DoctorIssue>,
    pub advice: Vec<String>,
    pub fixes:  Vec<FixAction>,
    pub duration: Duration,
}
// runner derivation:
//   outcome.status        = if issues.is_empty() { Ok } else { max_severity(&issues) → OutcomeStatus }
//   outcome.message       = report.summary (or synthesized from issues)
//   outcome.fix_available = !fixes.is_empty()
//   duration              = elapsed; panic → OutcomeStatus::Crash + Severity::Crash issue
```

`OutcomeStatus` (in `report.rs`) stays the stable *consumer* contract and gains a
peer `Crash` variant, but it is engine-derived; authors only ever name `Severity`.

```rust
pub enum OutcomeStatus { Ok, Warn, Error, Crash }   // derived rollup; not author-facing
```

## Always-on execution + visibility

- **Pre-launch gate** (`main.rs`, exists): runs the fast, **fail-open** subset and
  applies safe fixes silently before the tray comes up. Never blocks launch.
- **Dev-loop run**: a `"dev-loop"` group = `dev_link_paths`, `fingerprint_health`,
  `plugin_staleness`, `plugin_process_leaks`. The recompile/dev-build completion
  path runs `Selector::Group("dev-loop")` (replacing today's lone
  `PluginProcessLeaks` call in `restart_schedule.rs`). Drift surfaces the instant
  the build that caused it finishes.
- **Visibility policy (deep-module output):**
  - healthy → **nothing** (silence is the feature),
  - auto-fixed → one quiet "healed N" line,
  - unfixable → one actionable banner (issue + advice), via a generalized
    `BootHealedBanner` fed by doctor results.
  - **Stop suppressing doctor findings** — remove them from the
    `logging/control.rs` suppress-patterns; that suppression is *why nobody cares*.
- **Dev vs prod split (critical):** the end user never sees the doctor — prod heals
  silently per mission. The **dev** is the opposite: silent re-healing of the same
  dev-link/fingerprint drift every recompile hides a structural misconfiguration.
  `suppressed-errors.json` already counts repeats — use it: **dev escalates on
  recurrence** ("healed this 5× — your setup keeps drifting; root cause: …").
- **Guardrails:** pre-launch + dev-loop runs are **fail-open** (never block boot),
  **fast subset only** (the deferred cost axis keeps expensive/rebuild fixes out of
  the gate by construction), and **idempotent** (no fix-loops across iterations).
- The standalone `qol-tray-doctor` CLI/binary is **not** the hero surface (that's
  the thing nobody runs); it stays for CI/explicit use.

## Fix model (locked)

`FixApplicability` is a **safety/authority** axis only — never cost. The
`FixPolicy` gate is a threshold, so declaration order *is* the policy semantics.

```rust
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub enum FixApplicability { SafeAutomatic, ReversibleHostMutation, ManualOnly, Destructive }

impl FixAction { pub fn applicability(&self) -> FixApplicability { /* exhaustive match */ } }

pub struct FixPolicy { pub max_applicability: FixApplicability }   // FixCost added later (Deferred)
impl FixPolicy {
    pub fn safe()    -> Self { Self { max_applicability: SafeAutomatic } }
    pub fn startup() -> Self { Self { max_applicability: ReversibleHostMutation } }
    pub fn allows(&self, f: &FixAction) -> bool { f.applicability() <= self.max_applicability }
}
```

| FixAction | applicability |
|---|---|
| SetActiveInstallId, WriteInstallMarker, WriteAutostartEntry, EnsurePluginsDir, KillPluginProcessLeaks, InstallShellHook, RelocateDevLink | `SafeAutomatic` |
| UnshadowDeBinding, DisableSymbolicHotkey, ClearWindowsAppKey | `ReversibleHostMutation` |
| PruneOrphanFingerprints (new) | `SafeAutomatic` |

`auto_fix_startup()` uses `FixPolicy::startup()` → DE fixes still applied silently
(mission: take back the host surface silently + reversibly). `ManualOnly` /
`Destructive` have no constructor that enables them → they only surface as `advice`.

**No fix-ordering field anywhere.** `apply_fixes()` already applies fixes in
check-registry order (`CheckMeta.order` drives it transitively). Model a real
cross-fix dependency explicitly if one ever appears — not a flat int.

## fingerprint_health (integrity only)

Build-cache integrity, mirroring npm's `cacache.verify`. Must NOT report "needs
rebuild" — `plugin_staleness` owns drift; the file carries a comment saying so.

| Issue `code` | meaning | severity | action |
|---|---|---|---|
| `orphan` | fingerprint id ∉ registry | Warn | `PruneOrphanFingerprints { ids }` (SafeAutomatic) — the claude-sessions/kitty case |
| `phantom` | fingerprint present, no binary in workspace `target/{debug,release}` | Warn | `advice` → rebuild via dev-links panel / `cargo build -p <id>` |
| `corrupt` | file unparseable | Error | `advice` → reset (manual; never auto-clobbered) |

No `RebuildPlugin` fix — doctor prunes, the build subsystem builds (npm precedent).

## Crash isolation (runner boundary)

```rust
fn run_one(check: &dyn DoctorCheck, ctx: &DoctorContext) -> DoctorCheckResult {
    let start = Instant::now();
    let caught = std::panic::catch_unwind(AssertUnwindSafe(|| check.run(ctx)));
    let duration = start.elapsed();
    match caught {
        Ok(report) => derive_result(check.meta(), report, duration),
        Err(_)     => crash_result(check.meta().id, duration), // OutcomeStatus::Crash + Severity::Crash
    }
}
```

Safe: `run()` is read-only (fixes apply separately via `apply_fix`), and
`std::cell::OnceCell` is **not poisoned** by a panic in its init closure (unlike
`Mutex`/`LazyLock`), so `DoctorContext` stays reusable. Works because the
workspace builds with default `unwind` (no `panic = "abort"`). No generic timeout;
only the shell-out checks can wedge — bound *those* `Command` calls later.

## Staged plan (each stage: compiles, `fmt` + `clippy -D warnings` + tests green, ships alone)

**Stage 1 — Framework + registry + context + runner (emit-only, engine-derived verdict).**
`framework.rs` (trait, `CheckReport`, `CheckMeta` + builder defaults, `DoctorContext`,
`DoctorIssue`, `Severity`, `Selector`, the `catch_unwind`+duration runner that
*derives* `DoctorCheckResult`/`Outcome`). `OutcomeStatus` gains `Crash`. `checks.rs`
→ `registry() -> Vec<Box<dyn DoctorCheck>>` (cfg-gated dev pushes). Each
`checks/*.rs` returns `CheckReport` (mechanical: `ok_outcome`→`CheckReport::ok`;
`warn_outcome`→push issue + fix). Delete `CheckId` + `from_str` + the 2-arm match;
resolve single-run by `meta().id`. Keep existing `FixAction`/`is_safe_to_auto_apply`/
`FixPolicy{apply_de_fixes}` this stage. Update `cli.rs` (Crash label, exit 2),
`report.rs` (`Report` over results + `outcomes()` accessor), `restart_schedule.rs`
(`result.outcome.status`). **De-suppress doctor findings** in `logging/control.rs`.

**Stage 2 — fingerprint_health + dev-loop run (the feature; fast-path).** New
`checks/fingerprint_health.rs` (pure-fn core + map-probe, like `dev_link_paths.rs`).
Add `PruneOrphanFingerprints` (`is_safe_to_auto_apply == true`) + `apply_fix` arm.
Add the `"dev-loop"` group and run `Selector::Group("dev-loop")` from the recompile
completion path (replacing the lone `PluginProcessLeaks` call). Needs only Stage 1.

**Stage 3 — Mission-aligned fix levels.** Replace the bool + `FixPolicy{apply_de_fixes}`
with `applicability()` + `FixPolicy{max_applicability}` per the mapping.
`auto_fix_startup()` → `FixPolicy::startup()`. CLI `--apply-de-fixes` →
`--apply-host-fixes` (old as hidden alias). No `FixCost`.

**Stage 4 — Selection.** CLI `doctor check|fix [--id|--group|--category]
[--apply-host-fixes]`; runner filters by `Selector` + current platform + dev cfg.

**Stage 5 — Visibility surface (frontend).** Generalize `BootHealedBanner` into the
doctor result surface (silent / healed-N / actionable-unfixable); implement the
dev-vs-prod recurrence policy using `suppressed-errors.json` counts.

## Deferred (decisions recorded, code intentionally absent)

- **`FixCost { Cheap, Expensive }` + `FixPolicy.allow_expensive`** — only when the
  first genuinely expensive fix exists; then `allows()` gains
  `&& (allow_expensive || f.cost() == Cheap)` and a `doctor fix --allow-expensive`.
  Cost is orthogonal to safety; folding it into `FixApplicability` breaks the
  threshold total-order (an `ExpensiveAutomatic` tier either auto-runs minute-long
  builds at boot or ranks "expensive" as scarier than mutating the host).
- **`RebuildPlugin`** — advice-only now; if ever a fix, `{SafeAutomatic, Expensive}`, never auto.

## Per-file change table

| File | Change | Stage |
|---|---|---|
| `doctor/framework.rs` | **new** — trait, CheckReport, meta+builder, context, issue, severity, selector, runner+derivation | 1 |
| `doctor/checks.rs` | hardcoded vec → `registry()`; drop `collect_diagnosis` match | 1 |
| `doctor/checks/*.rs` (x8) | return `CheckReport`; add `impl DoctorCheck` | 1 |
| `doctor/mod.rs` | registry+selector+context wiring; remove `CheckId` | 1 |
| `doctor/report.rs` | `OutcomeStatus::Crash`; `Report` over results + `outcomes()` | 1 |
| `doctor/cli.rs` | Crash label+exit; (selector parsing → 4); (flag rename → 3) | 1/3/4 |
| `doctor/diagnosis.rs` | `Diagnosis` → derivation; `applicability()` (3); `PruneOrphanFingerprints` (2) | 1/2/3 |
| `logging/control.rs` | drop doctor findings from suppress-patterns | 1 |
| `features/.../restart_schedule.rs` | `result.outcome.status`; run `"dev-loop"` group (2) | 1/2 |
| `doctor/checks/fingerprint_health.rs` | **new** | 2 |
| `ui/components/BootHealedBanner.js` | generalize into doctor result surface | 5 |

## Testing

- Existing per-check pure-fn tests stay; trait wrappers are thin.
- New: registry ids unique; `Selector` filters by id/group/category/platform/dev;
  `catch_unwind` → `Crash` (not `Warn`); engine derivation (empty issues → Ok;
  max-severity rollup); applicability mapping table; fingerprint_health
  orphan/phantom/corrupt via map-probe.

## Decision Log

1. **Complex engine / simple interface** is the governing invariant (deep modules).
2. **Checks emit findings; engine derives the verdict.** Author never sets rolled-up
   `Outcome`/status/fix_available — that was an engine leak into the interface.
3. **One author-facing status enum (`Severity`).** `OutcomeStatus` is the derived
   consumer rollup, not author-facing.
4. **`CheckMeta.order` = display/exec only** (Flutter/RN/npm). Defaults so authors
   declare ~id+label.
5. **Keep `category` AND `groups`** (npm: multi-group; category = canonical home).
   RN's `common/android/ios` is platform partitioning → that's `PlatformScope`.
6. **Applicability levels, not safe/unsafe bool** (RN levels; rustc `Applicability`
   confidence). Confidence/safety ≠ cost → no `ExpensiveAutomatic` tier; cost is a
   deferred orthogonal axis.
7. **No `FixAction` priority/order (YAGNI).** Registry order already deterministic.
8. **`RebuildPlugin` → advice.** npm doctor verifies/prunes; build is the build tool's job.
9. **`Severity::Crash` + `OutcomeStatus::Crash`** (Flutter `ValidationType.crash`):
   a panicking check is a doctor defect, not a user fault.
10. **`fingerprint_health` = integrity, not staleness.** `plugin_staleness` owns drift.
11. **Always-on + visibility is the real lever**, not "run it more." Silent-healthy /
    loud-unfixable; de-suppress; dev-loop group on recompile; dev escalates on
    recurrence while prod heals silently. The standalone CLI is not the hero.
