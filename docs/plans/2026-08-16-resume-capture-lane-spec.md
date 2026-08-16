# Spec: lane resume capture (early external id, pi resume flag)

Two defects behind "an accidentally closed lane cannot be resumed":

1. The watcher records a lane's harness external id only at round completion
   (`watch.rs` completed branch) and at bridge collection, so a lane that dies
   mid-round leaves `external_id: null` in the spawn record and the respawn
   skips resume with `no_external_id`, even though the harness transcript
   survived on disk.
2. Pi resume uses `--session-id <id>`, documented as "Use exact project
   session ID, creating it if missing": any mismatch silently opens a brand-new
   chat. `--session <id>` resolves the session or fails loud
   ("No session found matching '<id>'", exit 1), verified against the installed
   pi binary, including resolution of a uuid recorded under another cwd.

All work happens in this worktree on branch `resume-capture`.
Scope: `tools/qol-cli/src/commands/sessions/spawn.rs`,
`tools/qol-cli/src/commands/sessions/watch.rs`,
`libs/qol-terminal-sessions/src/cli/builtins/pi/mod.rs`,
`libs/qol-terminal-sessions/src/cli/interpreter.rs` (test expectation only).
`bridge.rs` callers of `capture_lane_external_id` stay untouched; they may
ignore the new return value. Do not touch any other file.
Code comments are banned in this repo; use self-explanatory names.

## 1. Capture the external id while the round is still alive

`spawn.rs`: change `capture_lane_external_id` to return `bool`: `true` only
when it resolved an external id and `ledger.record` succeeded; every existing
early-return path returns `false`. Keep all probe! lines as they are.

`watch.rs`:

- Add field `external_id_captured: bool` to `WatchedRound`, initialized `false`
  in `WatchedRound::new`.
- In `poll_round`, immediately after a screen read succeeds (the point where
  the `screen` value exists, before the `screen.contains(&round.marker)`
  check), run:
  `if !round.external_id_captured { round.external_id_captured = super::spawn::capture_lane_external_id(terminals, interpreter, ledger, locks, &round.binding); }`
- Gate the existing completion-branch call the same way so a captured round
  does not re-discover: replace the bare call with the same
  `if !round.external_id_captured { ... }` assignment.

Tests (`watch.rs mod tests`, existing FakeBackend/facts/harness style):

- `lane_gone_before_completion_still_records_the_external_id`: build facts with
  `spawn_identity: Some(...)` (construct the same SpawnIdentity type the live
  spawn writes; find its constructor in qol-terminal-sessions),
  `foreground_basenames: vec!["pi".into()]`, `foreground_pids: vec![424242]`.
  Point pi session resolution at a temp dir by setting
  `PI_CODING_AGENT_SESSION_DIR` to a temp path containing
  `<encoded-cwd-dir>/2026-08-16T10-00-00-000Z_0000cafe-cafe-cafe-cafe-cafecafecafe.jsonl`
  (encoded dir name for cwd `/work` is `--work--`; the fake pid has no live
  process, so resolution falls back to the newest session file). Save and
  restore the env var exactly like
  `libs/qol-terminal-sessions/src/cli/builtins/pi/environment.rs` tests do.
  Screens: two idle reads, then the backend dies (existing `die_after_reads`
  seam) so the round goes `gone` without ever completing. Assert the gone
  event fired AND the ledger dir now holds a record whose `external_id` is
  `Some("0000cafe-cafe-cafe-cafe-cafecafecafe")`.
  If `CliSessionInterpreter::system()` cannot be made to resolve pi metadata
  from the fake facts, report that finding with the blocking detail instead of
  faking the assertion; do not weaken the test to "record exists".
- Keep `completed_without_a_spawn_identity_leaves_no_ledger_record` green
  unchanged.

## 2. Pi resumes with --session, never --session-id

- `pi/mod.rs` `resume_args`: `["--session", external_id]`.
- Update the expectation in `interpreter.rs` (the `("pi", ...)` tuple) and the
  two spawn.rs test sites currently expecting `"--session-id"`.
- Check `pi/tests.rs` for any other `--session-id` expectation and update it.

## Gate before committing

Run from the worktree root and paste real output in your report:

```
cargo test -p qol sessions
cargo test -p qol-terminal-sessions
cargo fmt --check -p qol -p qol-terminal-sessions
cargo clippy -p qol -p qol-terminal-sessions --all-targets -- -D warnings
```

Commit on this branch with a conventional message like
`fix(cli-sessions): capture lane resume ids while rounds are live`.
NEVER add Co-Authored-By, "Generated with", or any AI attribution to the
commit message.
