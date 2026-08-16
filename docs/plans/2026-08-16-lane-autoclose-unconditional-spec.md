# Spec: lane autoclose is unconditional

A completed lane must always deliver its report and close its terminal.
Today `autoclose: false` exists as a caller knob on session_spawn and
session_submit, and completed lanes sit open waiting for the driver to run
session_loop_close. Remove the knob entirely: the only sessions that are
never closed are ones without a spawn identity (the guard protecting real
user terminals stays exactly as it is).

All work happens in this worktree on branch `lane-autoclose`.
Scope: `tools/qol-cli/src/commands/sessions/mcp.rs`, `bridge.rs`,
`watch.rs`, and their tests. Do not touch any other file.
Code comments are banned in this repo; use self-explanatory names.

## mcp.rs

- session_spawn and session_submit REJECT an `autoclose` argument the same
  way session_spawn rejects `background`: a present `autoclose` key returns
  the error "`autoclose` was removed; lanes always close on completion".
  Delete the parsing and the threading of the flag.
- Update the help text: remove both autoclose sentences, state that lanes
  always close when the watcher confirms completion and that sessions
  without a spawn identity are never closed.
- Existing tests `session_submit_autoclose_opt_out_leaves_the_round_plain`
  and the spawn equivalent become rejection tests asserting the new error.
  `session_submit_never_autocloses_a_session_without_a_spawn_identity`
  stays and must keep passing.

## bridge.rs

- Drop the `autoclose: bool` parameter from the submit path. The stored
  round keeps its `autoclose` checkpoint field for on-disk compatibility,
  but its value is now derived only as `target.spawn_identity.is_some()`.
  Delete the `requested_autoclose` / `autoclose_forced_off` probe branch;
  there is no request to force off anymore.
- `PendingBridgeStore::start` keeps its boolean argument (it now means
  "this round targets a spawned lane").
- Update `submit_with_autoclose_stores_the_flag_and_forces_it_off_for_architects`:
  it becomes one test asserting a spawn-identity target stores true and a
  target without spawn identity stores false.
- `start_with_autoclose_roundtrips_and_old_checkpoints_default_to_false`
  keeps passing unchanged (old checkpoints still deserialize).

## watch.rs

- `wake_message` for `completed` no longer instructs anyone: drop the
  "Review it, then close the loop with session_loop_close." sentence and
  the conditional "(lane auto-closed)" suffix. New shape, closable round:
  "qol sessions: lane {session} completed and the lane terminal closed.
  Report below.\n\n{report}". Non-closable round (no spawn identity):
  "qol sessions: {session} completed. Report below.\n\n{report}".
  The `gone` and fallback arms are unchanged.
- The close attempt still runs only when the round is closable and the
  wake was delivered, exactly as the current `round.autoclose` gate does;
  only the name/meaning of the flag changes with bridge.rs.
- Update `autoclose_round_closes_the_lane_terminal_after_completed_and_plain_rounds_stay_open`:
  "plain" now means "no spawn identity"; a lane round always closes.
- Add a test: the completed wake text contains no "session_loop_close"
  and no "Review it".

## Gate before committing

Run from the worktree root and paste real output in your report:

```
cargo test -p qol-cli sessions
cargo fmt --check -p qol-cli
cargo clippy -p qol-cli --all-targets -- -D warnings
```

Commit on this branch with a conventional message like
`fix(cli-sessions): close lanes unconditionally on completion`.
NEVER add Co-Authored-By, "Generated with", or any AI attribution.
