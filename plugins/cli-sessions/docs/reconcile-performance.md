# CLI Sessions Reconcile Performance

Date: 2026-07-03
Worktree: `/Users/kaho/repos/private/worktrees/qol-monorepo/cli-sessions-reconcile-perf`
Branch: `cli-sessions-reconcile-perf`
Target: `plugins/cli-sessions`

Rule: every new performance test needs before and after numbers before it can justify an implementation change.

## Scope

- Reduce background reconcile cost without changing alert detection correctness.
- Focus first on service/process probing because the visible and hidden reconcile loops call it repeatedly.
- Keep alt-tab and other plugin worktrees untouched.

## Original Flow Map

- Startup loads config, converts configured service commands to shared state, builds one `SystemServiceProbe::snapshot`, then runs an initial `reconcile::tick`.
- The reconcile timer runs every 3s while the panel is visible and every 10s while hidden.
- Each timer tick builds a fresh `SystemServiceProbe::snapshot`, then calls `reconcile::tick`.
- `SystemServiceProbe::snapshot` shells out to `lsof -nP -iTCP -sTCP:LISTEN -Fp` and `ps -axo pid=,ppid=`.
- `reconcile::tick` discovers panes, prunes stale rows, reads screens for agent tools, asks the service probe only for generic non-prompt panes, then persists registry state.
- Existing correctness tests cover service classification through injected `ServiceProbe`; direct lazy-load tests were added with this change.

## Measurements

| Iteration | Scenario | Before | After | Delta | Correctness | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| 0 | Manual process-probe smoke, macOS, 5 repeats each of `lsof`, `ps`, combined shell pair | `lsof` 0.04s, `ps` 0.01s, pair 0.05s wall per run | `lsof` 0.04s, `ps` 0.01-0.02s, pair 0.05s wall per run | No material subprocess change | Accepted | Manual loop verified the scenario exercises the two subprocesses called by the process snapshot. |
| 1 | Scripted process-probe baseline, macOS, 30 repeats, `/tmp/cli_sessions_probe_measure.py` | `lsof` median 38.37ms, `ps` median 15.21ms, pair median 53.62ms / p95 54.81ms | `lsof` median 38.83ms, `ps` median 16.28ms, pair median 53.90ms / p95 55.53ms | Pair +0.28ms median; underlying command cost unchanged | Accepted; `cargo test -p plugin-cli-sessions --test reconcile --test strategy` passed before and after | Reports: `/tmp/cli_sessions_probe_baseline.json`, `/tmp/cli_sessions_probe_after_lazy.json`. The win is avoiding this pair until a generic pane needs subtree service detection. |
| 2 | Exact Rust constructor baseline, macOS, 20 repeats, temporary `tests/service_probe_perf_tmp.rs` | `SystemServiceProbe::snapshot(Vec::new())` mean 53.23ms | Mean 0.00ms at millisecond precision | About -53.23ms per constructor, effectively removing constructor subprocess cost | Accepted; service unit tests, exact perf test, reconcile tests, and strategy tests passed after | Confirms constructor no longer pays the subprocess pair before any service check. Temporary perf test was removed before final diff. |

## Visualization

Lower is better. One `█` is roughly 2ms.

```text
Exact Rust constructor, 20 repeats
Before  53.23ms | ███████████████████████████
After    0.00ms |
Delta  -53.23ms | avoided on construction

Underlying subprocess pair, 30 repeats
Before  53.62ms | ███████████████████████████
After   53.90ms | ███████████████████████████
Delta   +0.28ms | unchanged; now deferred until needed
```

Interpretation: this change does not make `lsof` or `ps` faster. It removes the unconditional cost from startup and every reconcile tick that has no generic pane needing subtree service detection.

## Rejected Or Invalid Runs

| Iteration | Scenario | Reason | Notes |
| --- | --- | --- | --- |
| 2 | First service unit-test run after adding loader-count tests | Invalid test design | Shared atomic counter was mutated by parallel tests; fixed by serializing those specific tests with a mutex and reran successfully. |

## Hypotheses

| ID | Hypothesis | Status | Evidence |
| --- | --- | --- | --- |
| H1 | Full-system service probing is a reconcile hot-path cost that can be cached or gated. | Accepted for lazy gating | Exact constructor baseline showed 53.23ms/run before; after lazy loading the constructor is below millisecond resolution and existing service correctness tests pass. |

## Limitations

- This pass measured the exact Rust constructor path and the underlying subprocess pair.
- It did not run a live daemon visible/hidden-panel soak test yet.
