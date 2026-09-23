# Sessions bridge stall: the silent wait and the abandoned attach

Investigated 2026-09-14 from the fork brief `sessions-spawn-stall-1789382899`.
The report answers six questions with file paths, log lines and timings, then records the fix that landed in this repo.

## Symptom

In pi session `01a09f41-3227-7474-89e0-dfd2d7d16f93` (log `~/.pi/agent/sessions/--media-kmrh47-WD_SN850X-Git-qol-monorepo--/2026-09-14T09-30-45-159Z_01a09f41-3227-7474-89e0-dfd2d7d16f93.jsonl`) the architect spawned one lane and bridged it.

- 10:45:24.353Z `session_spawn` (`key peak-rates-impl`), result 10:45:25.208Z: "spawned session v1:kitty:k10304_f00024.43:1302126 in the background".
- 10:45:27.955Z `qol-sessions_session_bridge` (tool call id in the transcript, arguments `{"session": "v1:kitty:k10304_f00024.43:1302126"}`).
- 10:47:15.361Z the tool result: `Failed to call tool: This operation was aborted`.
- 10:47:15.390Z the following assistant message: `stopReason = "error"`, `errorMessage = "This operation was aborted"`.
- The lane round completed at 10:48:09.410Z (receipt `~/.local/share/qol-tray/sessions/lanes/rounds/b3810614fa54f0b5360e80f1881f2c7b21feff425cd0db04a5222a77035c32a7/receipt.json`, `completed_at 2026-09-14T10:48:09.416Z`), and the watcher delivered the wake into the architect terminal at 10:48:10.299Z as a user message.

The wait lasted 107.4 seconds and the round needed 164.0 seconds. Nothing was deadlocked: the bridge was waiting correctly and the round finished 54 seconds after the human lost patience.
The visible defect is that the wait produced no output at all for those 107 seconds, so the only apparent escape was the interrupt, and the interrupt left state behind.

## What the bridge waits on

`session_bridge` (MCP) calls `bridge::resume` in `tools/qol-cli/src/commands/sessions/bridge.rs`, which first takes a per-session attach (an exclusive `flock` on `~/.local/share/qol-tray/sessions/pending-bridge/<sha256>.owner`, written with the owning pid), then waits.

For a pi lane (`session_is_pi`, detected through the transcript-pinned pi strategy) the wait is the pi-aware `wait_for_pi_round` in `bridge.rs`, and every iteration reads:

1. The pinned pi transcript, `~/.pi/agent/sessions/<project>/<timestamp>_<uuid>.jsonl`, through `CliSessionInterpreter::marked_report` and the pi strategy's `metadata::marked_terminal_text`.
   Only assistant text counts, and only after the user-message flush, so the echoed task prompt cannot be mistaken for a report.
2. The round checkpoint `~/.local/share/qol-tray/sessions/pending-bridge/<sha256>.json` (`session`, `driver`, `completion_marker`, `completed`, `closed`, `screen`, `autoclose`, `transcript_paths`).
   `completed` is set by whichever detector finishes first.
3. `terminals.discover()`, one `kitten @ ls` per iteration.
4. A kitty screen read (`kitten @ get-text`, `read_screen_relaxed`) only on the stall probe.

Poll cadence for the pi path: sleep 500 ms, doubling to a 1 s cap, reset to 500 ms whenever the transcript subscription ticks; cancel is polled at `CANCEL_POLL_INTERVAL` 250 ms.
Measured on a live lane: `reads=175` over `elapsed_ms=48151` (about four iterations per second), returning `completed:true` with the marker.

For a non-pi lane the wait is `wait_for_completion_with_backoff` in `libs/terminal-sessions/src/service.rs`, a kitty screen scrape whose backoff base is `WAIT_BACKOFF_BASE` 3 s with a 1 s cap (`next_backoff` saturates immediately), a strict full read every tenth iteration, and a match that requires the marker on two consecutive identical screens.

In parallel, the MCP server runs an in-process watcher (`tools/qol-cli/src/commands/sessions/watch.rs`, started by `McpSessionServer::run`) that polls every round at `POLL_BASE` 3 s growing to `POLL_CAP` 5 s, writes the receipt, autocloses a spawned lane and delivers the wake into the driver terminal.
That cadence is what the incident trace shows: kitty reads every 3.01 s, stretching to 5.01 s under backoff, and `CLI_SESSION_WATCH event=completed ... reads=48` over 164 seconds.

## Every timeout on the path

| Layer | Value | Fires first? | Caller sees |
|---|---|---|---|
| pi MCP request timeout (`~/.pi/agent/mcp.json`, `"requestTimeoutMs": 86400000`) | 24 h | no | nothing for 24 h |
| MCP round timeout (`mcp.rs`, `TIMEOUT_MAX_MS`) | 24 h | no | `completed:false` outcome |
| Bridge wait `timeout` argument (CLI clamps to 1 s .. 24 h; the MCP tool refuses `timeout_ms`) | 24 h in practice | no | `completed:false` outcome |
| Stall probe `STALL_PROBE_AFTER` | 30 s | only when the interpreter reports no activity (or reports nothing for 4 x 30 s) | `stalled:true` plus a recovery command |
| `delivery_observed` window at resume start | 15 s, 1 s steps | on a silent lane, before the wait even begins | up to 15 s of extra silence |
| kitty command timeout (`libs/terminal-sessions/src/kitty/mod.rs`, `COMMAND_TIMEOUT`) | 30 s per call | only if `kitten` hangs | cancel and stall checks delayed up to 30 s |
| Cancel poll (`CANCEL_POLL_INTERVAL`, `WAIT_CANCEL_POLL_INTERVAL`) | 250 ms | n/a | the floor for reacting to `notifications/cancelled` |

On this path none of them fired.
The client aborted the request and the server never learned, which is the whole mechanism below.

## What an interrupt does

Pressing Esc in pi aborts the harness signal.
For an MCP direct tool that abort is observed by the pi MCP adapter's own wrapper (`~/.pi/agent/npm/node_modules/pi-mcp-adapter/direct-tools.ts:532`, `abortable(conn.client.callTool(...))`), which is where `Failed to call tool: This operation was aborted` comes from.
Reproduced with the same client stack (`@modelcontextprotocol/client` 2.0.0 and `StdioClientTransport`, the versions pi uses): the `callTool` promise is not rejected by the signal at all.

- Abort issued at 11:13:10.042, eight seconds into the bridge.
- The request promise resolved at 11:13:53.974 with the round's report, 43.9 s later, when the lane finished, with nobody awaiting it.

The Rust side never sees a cancellation because neither the adapter nor the SDK sends `notifications/cancelled` on an aborted request (the only occurrence of that notification in the SDK client is the client's own subscription teardown).
The server does implement the notification (`tools/qol-cli/src/commands/sessions/mcp.rs`, `dispatch_line`, `notifications/cancelled` -> the request's `AtomicBool`), and when it arrives the wait stops within one cancel poll and releases the attach.

Because the abort is invisible, the abandoned worker keeps waiting and keeps holding the per-session attach.
That is the state the incident left behind: `sessions_list` still showed the lane busy, the round open, and no architect waiting.

## Reproduction

Protocol-level driver: run `qol sessions mcp` over stdio and drive raw JSON-RPC, in the same shape pi's adapter uses.
A raw socket-level client is enough to control both the abandonment and the notification:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"probe","version":"0"}}}
{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"session_spawn","arguments":{"tool":"pi","cwd":"/tmp/scratch","key":"probe","task":"write 24 small files, one per tool call"}}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"session_bridge","arguments":{"session":"v1:kitty:..."}}}
{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":3,"reason":"probe abort"}}
```

Send request 3, wait a few seconds, then either send nothing (abandonment) or send the cancellation, and inspect the response to request 3 plus the result of a second `session_bridge` (file a later request with a new id).
The probes used pi lanes in a scratch cwd (`/tmp/qol-stall-probe/scratch`), each with a bounded task of 24 file writes so a round runs 40 to 60 seconds.

Abandoned attach, before the fix (`qol` from PATH, 2026-09-14 09:07 build):

```
[10:53:59] session token: v1:kitty:k10304_f00024.45:1340507
[10:53:59] bridge#1 sent id=3; abandoning it after 8s without any cancellation
[10:54:07] bridge#2 (no cancel sent): another bridge process (pid 1340487) is already attached to
           `v1:kitty:k10304_f00024.45:1340507`; never start a second one - run `qol sessions next ...`
```

The pid in that message is the probe's own MCP server process, so the holder is the abandoned request in the same process the caller is talking to.

Every documented recovery is blocked in that window (`PROBE_MODE=recovery`, 136 seconds into a live round):

```
qol sessions next   -> exit 0: phase=attached ... Another bridge process (pid 1510286) is already
                       attached to this session and is waiting for its completion signal. Do not
                       start a bridge, resume, or new task for this session.
qol sessions resume -> FAILED: another bridge process (pid 1510286) is already attached ...
qol sessions bridge -> FAILED: another bridge process (pid 1510286) is already attached ...
```

Control, same lane, with an explicit `notifications/cancelled`:

```
[10:59:33] bridge#1 after cancel: {"result":{"content":[{"type":"text","text":"bridge wait cancelled;
           the round stays open and recoverable via `qol sessions resume`"}],"isError":true}}
[11:00:02] bridge#2 (after cancel): {"completed":true,...,"completion_marker":"QOL_BRIDGE_DONE_f8e6..."}
```

The cancellation path works end to end in 220 ms, releases the attach, leaves the round open, and the next bridge serves it.
The defect is only that the harness never uses it.

Failure modes checked:

- Lane exits before the marker: the watcher writes a markerless receipt (`"markerless":true`) and the wake still fires (observed on the first probe lane, which died on a provider error).
- Lane writes a marker the watcher does not match: not observed for pi, because the transcript path accepts the token anywhere in assistant text and the screen path tolerates wrapped or split markers (`screen_contains_ignoring_whitespace`, `marker_close`).
- Two bridges in one pi session, and bridge after a previous aborted bridge: the abandoned-attach case above; fixed here.
- Lane tab closed or renamed mid-bridge: `session_gone` plus `transcript_report` recovers the round from the transcript when the terminal disappears (existing coverage: `the_pi_wait_rescues_a_finished_lane_whose_terminal_closed`).
- kitty remote control losing the route mid-wait: the pi wait degrades rather than fails, because a failed discovery keeps the last transcript paths and a failed screen read only empties the stall probe's snapshot, while the non-pi screen wait propagates `bridge screen read failed` and leaves the round open. Not reproduced against a live kitty loss.
- Ledger consistency after an aborted bridge: the checkpoint keeps the round open and the receipt is still written; no evidence of a stuck-open round after completion. The probe lanes left six orphaned checkpoints behind, and each cleared with `qol sessions discard`.

## Root cause

1. A `session_bridge` wait holds a per-session attach for its whole lifetime, and the attach is only released on completion or on an explicit cancellation the harness never sends.
   A client-abandoned request therefore becomes a phantom attach that outlives the caller's interest, and it refuses every later bridge or resume for that session, in every process.
2. The wait itself is silent from the harness's point of view.
   The MCP server emits `notifications/progress` every 30 s, but only when the request carries `_meta.progressToken` (`mcp.rs`, `dispatch_line`), and pi's MCP adapter only wires `onprogress` for proxy-mode calls (`pi-mcp-adapter/proxy-modes.ts:49` used at `proxy-modes.ts:1292`); the direct-tool path calls `callTool` with no progress callback (`pi-mcp-adapter/direct-tools.ts:532`).
   So a 107-second wait and a deadlock look identical in the TUI.
   That half is not fixable on the server side: MCP requires the client to supply the progress token.

## The fix

`tools/qol-cli/src/commands/sessions/bridge.rs`

- `PendingBridgeStore` now keeps a process-local registry of live attaches (`BridgeAttach { cancel, superseded }`) keyed by session token, and `acquire_owner` takes the request's cancel flag.
- On an owner conflict, if a local attach exists it is signalled (its cancel flag is set, so its wait stops within one 250 ms cancel poll and releases the flock), and the new caller retries the lock under a bounded budget (`ATTACH_TAKEOVER_BUDGET` 10 s, polled every 25 ms).
- A conflict whose owner file names this process is retried inside the same budget, which covers an attach that has claimed the lock but not yet registered.
- Conflicts held by another process, and conflicts with no cancel flag at all (the CLI paths), keep the previous behaviour and the previous message.
- `BridgeOwner::drop` removes its registry entry before clearing the owner file and unlocking, so a takeover can never observe a half-released attach.

`tools/qol-cli/src/commands/sessions/mcp.rs`

- The tool dispatch chain passes the request's `Arc<AtomicBool>` down to `session_bridge`, so the attach and the `notifications/cancelled` handler share one flag.
- `session_spawn` keeps its borrowed flag through `as_deref()`.

Regression tests:

- `a_new_attach_supersedes_a_local_one_instead_of_conflicting` (bridge.rs): a newer attach signals the older one, the older reports `superseded()`, and the newer one owns the session after the older releases.
- `a_second_bridge_in_one_process_supersedes_the_abandoned_attach` (mcp.rs): a second `session_bridge` in one server supersedes the first with the superseded message, keeps the round open, and then completes it with the report.

Gate, on the branch with the change:

```
cargo fmt -p qol --check                                     clean
cargo clippy -p qol --all-targets -- -D warnings              clean
cargo test -p qol --bin qol                                   1357 passed, 0 failed
cargo test -p qol-terminal-sessions                           268 passed, 6 passed, 0 failed
```

Live verification against the rebuilt binary (`target/debug/qol`), same probe as the failing case:

```
[11:21:09] bridge#1 sent id=3; abandoning it after 8s without any cancellation
[11:22:34] bridge#2 (no cancel sent): {"completed":true,...,"screen":"Done: `./notes` exists with
           exactly 24 files ... QOL_BRIDGE_DONE_c932c972075cdb1d69eb"}
```

The trace shows `CLI_SESSION_BRIDGE event=owner_superseded` once per supersede, and the CLI paths (`next`, `resume`, `bridge` from another process) still refuse while a foreign attach is live, which is the intended cross-process behaviour.

## Harness-side finding, not fixed here

`pi-mcp-adapter` (npm 2.32.1, installed at `~/.pi/agent/npm/node_modules/pi-mcp-adapter`) never asks for progress on direct MCP tools:

- `direct-tools.ts:532` builds `requestOptions` from the server manager and calls `callTool` with no `onprogress`.
- `proxy-modes.ts:49` has `withUiProgressBridge`, which does set `onprogress` and lets the SDK inject `_meta.progressToken`, but it is only used on the proxy path (`proxy-modes.ts:1292`).

Adding `withUiProgressBridge` to the direct-tool call would light up the existing 30 s `notifications/progress` stream from `qol sessions mcp` and turn a silent wait into a visible one.
That is an upstream package fix, so it is reported here and left unimplemented.

Related, from the same stack: `@modelcontextprotocol/client` does not send `notifications/cancelled` when a stdio request's signal aborts, and does not reject the request promise either.
If it did, the qol server would release the attach within 250 ms and none of the above would need a takeover.
The qol-skills plugin's own `session_bridge` tool (`extensions/hooks.ts`, which runs `qol sessions bridge` as a child and kills it with `SIGTERM` on abort) does cancel correctly, but it is a different process and is refused while an attach from the MCP surface is live.

## Failure surface left behind

- Cross-process recovery during the window: `qol sessions bridge` and `qol sessions resume` from another process still fail with `another bridge process (pid N) is already attached` while an abandoned attach lives in the MCP server.
  The harness's own surface recovers now, and `qol sessions next` keeps printing the accurate instruction, so this is a bounded degradation until the round completes.
- Cancel and supersede latency: up to 15 s while inside `delivery_observed`, up to 30 s while a single kitty command is in flight, 250 ms otherwise.
- The installed `qol` in `~/.cargo/bin` is a copy, not a link to `target/`; `qol setup` refreshes it (staged copy plus rename, no tray restart).
  Running pi sessions keep their old `qol sessions mcp` child until the harness restarts the server, so the fix reaches new sessions first.
- Incidental, verified while reproducing: the kitty backend cannot spawn when the MCP server is started with a sanitized environment (the SDK's `getDefaultEnvironment` drops `KITTY_LISTEN_ON` and `XDG_*`), which surfaces as `terminal backend refused the spawn request`. pi is unaffected because the adapter passes the full environment (`server-manager.ts`, `resolveEnv`).
