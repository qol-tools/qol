# CLI Sessions performance

The screen cache retains one immutable analysis per live pane: raw text, normalized activity hash, screen evidence, pane facts, and harness identity. A subscribed cache hit returns an `Arc` rather than copying the screen. A fresh read with identical text, facts, and tool reuses analysis. Any raw-text change invalidates analysis even if normalized activity hashes match, so footer changes cannot hide new screen evidence.

Active polling, subscription notifications, fallback reads, descriptor lifecycle evidence, and the attention reducer retain their existing cadence. Changed pane facts force a fresh read; changed process or external session identity replaces the cache. Failed reads provide no current screen evidence and retry. Removing a pane removes its analysis.

The existing `CLI_SESSIONS_RECON` screen-read trace includes `analysis_reused` to distinguish terminal I/O from repeated parsing. No screen contents are added to traces.

Run the synthetic analysis benchmark with:

```sh
cargo test -p plugin-cli-sessions --lib benchmark_unchanged_screen_analysis -- --ignored --nocapture
```

The `SCREEN_ANALYSIS_BENCHMARK` JSON reports elapsed nanoseconds, iteration counts, harnesses, and sample sizes. Both paths include allocation of a fresh input string; the cached path retains the previous analysis. This isolates analysis work, excluding terminal IPC, discovery, persistence, and rendering. Debug results are not a claim about release performance or overall CPU usage.

Activity rings share one `qol-gpui::activity_animation::ActivityAnimation` clock per surface at a maximum scheduled cadence of 30 FPS, rather than requesting every display refresh from each dot. Rings derive their phase from elapsed time, so skipped frames do not slow the cycle. Waiting-only, hidden, and non-animated collapsed surfaces do not retain the clock. Polling and keyboard updates remain independent of animation timing.

Measure a live Linux process without attaching a profiler or modifying it:

```sh
node plugins/cli-sessions/scripts/measure-process.mjs PID 60 target/cli-sessions-performance/process.json
```

The report records executable identity, per-second samples, CPU normalized to one core, CPU charged to reaped children, and resident memory. Compare the same build profile, window visibility, and session workload. A restart resets allocator state, so a memory decrease alone does not prove a rendering optimization. Existing reconciliation traces cover polling; CPU samples and the shared clock's bounded timer cover animation scheduling without adding a per-frame log stream.
