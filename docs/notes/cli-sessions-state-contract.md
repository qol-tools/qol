# CLI Sessions state contract

`Status::definition()` in `plugins/cli-sessions/src/session/status.rs` owns the exhaustive mapping from session state to label, sort priority, human-attention policy, idle policy, and shared palette roles. Row dots, collapsed-panel priority, counters, notifications, and registry summaries consume this definition. `tests/state_mapping.rs` verifies every state against both native theme palettes.

Harness identity and session state are separate signals. Harness badges consume `CliTool.label` and `CliTool.accent` from `qol-terminal-sessions`; status dots consume semantic colors from `qol-theme`. Render both through the shared GPUI kit. Never substitute a harness accent for a status color or copy RGB values into a plugin.

Human input or approval takes precedence over bridge state. An open driver loop is coordinating; a completed delegated lane awaits agent review. Neither requests human attention. Preserve the underlying runtime status separately so closing a loop and acknowledging a session do not corrupt activity tracking.

The shared information role identifies orchestration and services, success identifies active work, warning identifies a completed turn awaiting the user, danger identifies an explicit request for input, and muted text identifies idle or acknowledged sessions. Theme selection colors remain independent of runtime state.
