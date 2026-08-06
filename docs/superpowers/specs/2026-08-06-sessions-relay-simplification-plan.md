# Sessions relay: submit fix and surface simplification — plan

Date: 2026-08-06
Status: executed on branch `relay-simplify` (worktree); reviewed; pending merge

## Review round (2026-08-06, qol-code-review board, verdict conditional)

Six-reviewer board (correctness, security, tests-qa, architecture primaries;
adversarial + quick-wins passes) over `main...relay-simplify` in both repos.
No blockers or highs after the adversarial pass. Must-fix items landed as
commits `37434ad7`, `f7cab7e5`, `f9d753ac` (qol-monorepo) and `5f0d4e0`
(qol-skills):

- Two-write submit: `--bracketed-paste=auto` payload, target re-validation,
  then `--bracketed-paste=disable` carriage-return write. Restores kitty's
  paste sanitization for bracketed-paste-enabled targets (nvim/readline),
  narrows the process-change gap between writes (a TOCTOU window remains
  between the second `@ ls` and the `\r`), and restores paste semantics
  for multi-line payloads on paste-aware targets.
- Bounded kitty wait: the backend kills the kitten child after 30s instead of
  blocking callers forever on a wedged control socket.
- Verify node: failure branch reachable (exec capture inside an if), headless
  count asserted zero, run-id hex sequence, real inspect command on failure.
- Tests: stale-token submit test, failing-send isError test, row-shape
  pinning (backend/native present, pending_input/reported_cmd absent).
- mcp args rejected with usage; mode_label reused; WAIT_TIMEOUT_MIN_MS const.
- qol-skills: kimi stays manual MCP registration (no mcpServers in its
  manifest); skill states failed sends error the tool.

Deferred (tracked): security-2 /tmp last-send fallback hardening; client-side
qol-terminal-telepathy prune after merge+push (arch-1); plan-log completeness
fix applied here; trailing-\r payload normalization (correctness-5).

Re-verification after fixes: guest node pass on kitty 0.32.2
(`verify/reports/linux-mint-cinnamon-18c929dcff86171a-c95a8-1/report.json`):
zero qol processes, submit into the unfocused python window executed
(py settled True, 451ms), echo-exclusion settled (sh settled True).

## Execution log (2026-08-06)

- Commit `a4c104b1` (qol-monorepo): submit fix in
  `libs/qol-terminal-sessions/src/kitty/mod.rs` — `DeliveryMode::Submit` is now
  one `send-text --bracketed-paste=disable` write carrying the payload plus a
  trailing carriage return; the intermediate re-validation discovery and the
  `send-key` step are gone. Insert mode unchanged. `validate_target` removed
  (Submit-only). Tests: `submit_writes_text_with_carriage_return_unbracketed`
  and `insert_stays_bracketed_without_submit`; 47 lib tests green, clippy and
  fmt clean.
- Commit `f2f7f63c` (qol-monorepo): MCP delivery is synchronous — the
  per-session `DeliveryQueue` (worker thread, cap 8, `pending_input`,
  `last_error`) is deleted; CLI and MCP list rows come from one builder in
  `contract.rs` (drops the drifted `reported_cmd`/`pending_input` fields,
  adds `backend`/`native`); `qol sessions mcp --help` / `-h` / `help` print
  the server usage instead of hanging or failing as an unknown help topic.
  Net -217 lines. 34 sessions tests green, clippy and fmt clean; pi export
  byte-identical to the checked-in hooks.ts.
- Commit `a358cb4` (qol-skills): telepathy merged into the qol-sessions skill
  (one relay doc, unioned triggers, drift fixes: `qol sessions mcp --help`
  advice replaced with `qol sessions mcp help`, `pending_input` claims
  dropped, codex registration wording softened); telepathy plugin deleted;
  manifests resynced; qol-sessions bumped to 0.2.0. Sync tests 35/35.
- Guest verification (this branch adds `verify/sessions-relay-verify.sh`, a
  repeatable node: worktree in, verdict report out):
  `verify/reports/linux-mint-cinnamon-18c92824e03f3d8e-a0bb4-1/report.json`
  status pass. On kitty 0.32.2 with two windows: zero qol processes (no tray,
  no cli-sessions daemon, no socket), submit into the UNFOCUSED python window
  executed (`print(6*7)` -> 42, wait settled in ~315ms), echo-exclusion wait
  on bash settled (`echo telepathy-ok`).

## Context

The agent-relay feature (`qol sessions` CLI + MCP server + pi tools, skills in
qol-skills) was verified live in a clean Mint 22.3 guest VM (2026-08-06).
Two outcomes:

1. **The headless architecture holds.** The relay works with zero qol
   processes: no qol-tray, no cli-sessions daemon, no plugin socket. This
   settles the "integrate into cli-sessions" question: the plugin daemon is
   tray-bound and GPUI-only; a plugin-hosted relay cannot exist on a machine
   where the tray is not running. The standalone surface stays.
2. **A delivery defect was found.** `DeliveryMode::Submit` (insert text, then
   `kitten @ send-key --match id:N enter`) silently misfires whenever the
   target window is not the active window. Root cause: kitty <= 0.32.2
   `kitty/rc/send_key.py` calls `windows_for_payload` without
   `window_match_name='match'`, so the payload's match field is never read and
   the key goes to `boss.active_window`. Fixed upstream in kitty v0.33.0.
   Ubuntu 24.04 / Mint 22.x ship 0.32.2, so every stock build is affected.
   The original guest verification missed it because it used a single window
   (target == active window).

## Research verdicts (2026-08-06, 4 parallel agents)

| Question | Verdict | Evidence |
|---|---|---|
| Submit fix shape | `send-text --bracketed-paste=disable` with `text + "\r"` in one write. Works on 0.32.2 and 0.45.0; no focus changes; no version gate | Live tests on real kitty windows: bash, python REPL, Claude Code, pi (codex inferred). `\r` is the byte Enter produces in raw-mode TUIs |
| Trailing `\n` inside `--bracketed-paste=auto` | Rejected: readline inserts the paste literally and never accepts; python 3.12 only "works" because it disables bracketed paste (bpo-42819) | readline 8.2 `rl_bracketed_paste_begin` + live tests |
| Focus-before-enter | Rejected: steals OS focus, races with user input, disruptive for voice dictation (voice never focuses targets) | kitty sources + voice delivery path |
| `pending_input` / DeliveryQueue | Zero consumers anywhere (hooks.ts, skills, contract). Safe to drop | repo-wide grep |
| Row-builder drift | CLI and MCP list rows already drifted (`reported_cmd` vs `backend`/`native`/`pending_input`) | mod.rs vs mcp.rs |
| `qol sessions mcp --help` | Broken: dispatch discards args, `mcp::run()` blocks on stdin; help extractor exits 64 "Unknown help topic" | dispatch trace |
| Skills merge | Full merge into qol-sessions plugin; delete telepathy plugin; sync script regenerates all manifests; no script changes needed | qol-skills structure + sync script |

## Track 1: Submit fix (correctness, shared lib)

The fix lives in `libs/qol-terminal-sessions` so all consumers inherit it:
`qol sessions send --submit`, MCP `session_send_text submit:true`, and
qol-voice's opt-in Submit mode (default is Insert, unaffected).

1. `libs/qol-terminal-sessions/src/kitty/mod.rs` (`TextInput for KittyBackend`,
   lines 177-211): Submit path becomes one atomic write —
   `kitten @ send-text --match id:N --stdin --bracketed-paste=disable` with
   payload `text + "\r"`. Delete the intermediate `validate_target` +
   `send-key` step: the write is atomic, so the re-validation race it guarded
   against no longer exists. Keep the existing `ensure_capability` discovery
   and the empty-text early return.
2. Insert mode stays `--bracketed-paste=auto` (paste-aware targets keep paste
   semantics).
3. Update the sequence test `submit_validates_before_text_and_before_enter`
   (lines 298-330): assert one `@ ls` + one `send-text` carrying `cargo test\r`
   with `--bracketed-paste=disable`. Update the `ls()` fixture only if needed.
4. Grep for other tests asserting the two-phase sequence (mcp.rs tests use
   FakeBackend, not the kitty sequence; verify during implementation).

## Track 2: Surface simplification

### 2a. qol-cli sessions group (`tools/qol-cli/src/commands/sessions/`)

5. `mcp.rs`: delete `QueueState`/`DeliveryQueue` (lines 29-168), the queue
   field and its construction, `QUEUE_CAPACITY_PER_SESSION`, and now-unused
   imports. `tool_send_text` (339-361) calls `terminals.send_text` directly,
   returns `delivered {verb} to {binding}` (CLI wording).
   `tool_wait_output` (363-388) drops `wait_for_empty`/`take_last_error`;
   `tool_list_sessions` drops `pending_input`.
   Tradeoff: a slow kitty IPC write now blocks the single-threaded server
   loop; MCP clients block on tool calls anyway, and the CLI path already
   behaves this way.
6. `contract.rs`: add the shared `SessionRow` Serialize struct + builder and
   the single `capability_names` fn (one type: `Vec<&'static str>`).
   Unified row: `session, backend, native, root_pid, cwd, title, at_prompt,
   tool, display_name, activity, capabilities`. Drop `reported_cmd` and
   `pending_input`. Preserve the five field names hooks.ts consumes
   (`session, tool, display_name, cwd, activity`).
7. `mod.rs`: `list()` (87-131) uses the shared builder (keeps token sort +
   pretty print); delete local `SessionRow` (313-322) and local
   `capability_names` (291-309). `mcp.rs` `tool_list_sessions` (303-328) uses
   the shared builder; delete local `capability_names` (495-507).
8. `--help` fix: `mod.rs` line 45-46 passes `rest` to `mcp::run(rest)`;
   `mcp.rs::run` (399-410) prints usage for `help|-h|--help` before the stdin
   loop (copy the `export` subcommand pattern, export/mod.rs 9-21). Fix the
   stale queue claim in `help_text()`.
9. Tests: rewrite `send_text_tool_delivers_with_submit_mode` (711) and
   `send_text_tool_queues_in_fifo_order` (739) for sync delivery; delete
   `delivery_queue_worker_delivers_in_background_in_order` (779) and
   `send_text_tool_rejects_when_queue_is_full` (807); edit the two
   `wait_output` tests that call `drain_manual` (924, 1003) and the list-row
   test (689).

### 2b. qol-skills consolidation

10. Merge `plugins/qol-terminal-telepathy/skills/qol-terminal-telepathy/
    SKILL.md` into `plugins/qol-sessions/skills/qol-sessions/SKILL.md`:
    union the frontmatter trigger vocabulary (relay, handoff, your turn,
    wait-for-agent, ping-pong, tool reference); keep the 5-tool table, wait
    semantics, the 8-step relay loop, idle signals/title formats, when-to-
    wait-vs-send, when-NOT-to-use, setup-once, smoke test, failure grammar,
    safety rules, constraints. Drop both cross-references and the broken
    `qol sessions mcp --help` availability check (replace with
    `qol sessions help` / `qol sessions export`). Soften the codex
    manual-registration claim (`.codex-plugin/plugin.json` now declares
    mcpServers).
11. Delete `plugins/qol-terminal-telepathy/` (all 5 files). Nothing else
    references it.
12. Bump `plugins/qol-sessions/` to 0.2.0 in the four plugin manifests; run
    `node scripts/sync-plugin-manifests.cjs` (regenerates marketplaces,
    `kimi.plugin.json`, root `package.json`; telepathy entries vanish
    automatically). No sync-script code changes needed.
13. Client-side: the user's installed-plugins config referencing
    qol-terminal-telepathy must be pruned (marketplace three gates).

## Out of scope (deferred, documented)

- Exposing cli-sessions status (YourTurn/NeedsYou) through the sessions
  surface (design-spec status-surface item; requires a daemon socket reply
  path — separate decision).
- Turn-engine consolidation (cli-sessions status machine, qol-voice
  coordinator, telepathy procedure).
- kitty version requirement bump (the fix works on every version; a doc note
  that kitty >= 0.33 removes the underlying bug is enough).

## Verification

1. Host gates: `cargo build` + focused tests for `qol-terminal-sessions` and
   `qol-cli`, `cargo fmt`, `cargo clippy -D warnings`; `qol sessions help`
   smoke; `qol sessions export pi` output diffed against checked-in hooks.ts.
2. qol-skills: `node scripts/sync-plugin-manifests.cjs --check`; pi extension
   load via `pi -p`.
3. Guest VM (the decisive check): script the manual loop from the 2026-08-06
   verification into a repeatable workflow node (input: worktree; output:
   verdict + report). Scenario on kitty 0.32.2 with two windows: headless
   proof (no tray processes), submit into the UNFOCUSED target (the defect
   repro: `print(6*7)` must execute without any manual focus step),
   echo-exclusion wait on bash (`echo telepathy-ok`), JSON contract checks.
4. Host-side parity spot check with kitty 0.45.0 if the user approves host
   verification; otherwise the guest covers the contract.

## Risks

- `--bracketed-paste=disable` changes paste semantics for paste-aware targets
  (bash readline): text is now typed-style, not paste-style. Live-verified
  correct for submit; Insert mode is unchanged.
- Multi-line payload + submit: readline accepts the whole buffer on the final
  `\r`; python executes line-by-line. Same as human typing.
- Sync MCP delivery blocks the server during slow IPC; accepted (clients
  block anyway).
- The marketplace cache serves stale qol-sessions until the qol-skills bump
  is pushed (push is user-gated per repo policy).

## Delivery

- qol-monorepo, direct to main: commit 1 = submit fix (+ tests);
  commit 2 = MCP/surface simplification. Push only when asked.
- qol-skills, direct to main: commit = skill merge + plugin deletion + sync +
  version bump. Push only when asked (required for the marketplace cache to
  refresh).
