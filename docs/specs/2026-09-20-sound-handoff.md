# Sound implementation handoff, 20 September 2026

## Stop state and first read

**Status update, same day, after the handoff was written.** The handoff was resumed. The two proof assertion defects under "Immediate unfinished correction" are fixed and the sixth round passed on a fresh guest: leaf SHA `e1ac3810e5e1313a4f4bda6c4fe371207fa0a3747bd0c0e4d0bc3f0b359a0506`, report `/tmp/code-review/sound-runtime-proof-20260920/native-consumers-round-six.json`, all 26 pass conditions true, cleanup ok, guest torn down. That section below is history; do not redo it. The spec `2026-09-20-sound-runtime-fixes.md` carries the details at the top. Remaining work starts at item 2 of "Full product work still required".

The user stopped the outgoing agent and requested this handoff. Do not interpret this document as a request for the outgoing agent to continue. No implementation or tests were resumed after that instruction. No Sound lanes are running. The final idle MCP transport was closed. The last guest was stopped with verified cleanup; no development guests remained at the last check.

The authorized product is the complete Sound plugin, bundled audio client, existing audio-consumer migrations, ownership/release integration and the remaining design/runtime fixes. This is not complete. The useful implemented subset is the tray reload race fix, native Pulse client, and Shot/Voice/Bluetooth transport migrations. Sound itself and its native PipeWire/ownership backend remain to be implemented. That was known from the outset; do not spend scouts rediscovering whether Sound exists. Read and reuse the existing design and research.

Start with these sources:

1. This handoff.
2. `docs/specs/2026-09-20-sound-runtime-fixes.md` in the worktree below. It contains exact ownership, frozen APIs, acceptance gates and recent correction instructions, newest evidence near the top. Older sections preserve history and must not override newer results.
3. The accepted design snapshot `/tmp/code-review/sound-design-fixes-after-v33/after/arch.mjs`, SHA256 `2bbfe1e934a5d62ebfcf2688da4a90f902f6cce57e4c32bec88ee46d8abeb46e`. The original user-facing sheet is `settings/Settings Grounds And Depth.html`; the accepted local snapshot was not republished to it.
4. `/tmp/code-review/sound-runtime-proof-20260920/audio-round-four-check.json` and `native-consumers-round-five.json` for the actual current validation state.

## Repository and authorization

- Main checkout: `/media/kmrh47/WD_SN850X/Git/qol-monorepo`. Clean when this handoff was written.
- All implementation: `/media/kmrh47/WD_SN850X/Git/worktrees/sound-runtime-fixes/qol-monorepo`.
- Branch: `sound-runtime-fixes`.
- Worktree HEAD and source baseline: `479eba996ce48165504f88be36c28da9fc1d81fb` (`docs(bluetooth): hand off the paused-channel link repair`).
- No implementation commits, merges or pushes were made. The entire implementation is an uncommitted worktree diff, including untracked new directories. Preserve it.
- The user explicitly invoked `$qol-sessions:architect`. Under that skill the architect writes plans/specs, personally reviews code and runs central verification; terminal lanes make semantic implementation edits. No in-harness subagents for lane work. Apply the current skills when resuming, unless the user changes the role.
- Lanes were edit-only with exact disjoint ownership, no build/test/lint/format/git/runtime commands, no source comments, no em-dash, no subagents. The architect centrally formats only explicitly owned files and runs the gate after all writers finish.
- Runtime testing is in disposable headless offline guests, not the user's host audio/desktop. No host configuration changes, new cross-plugin broker or workflow-tool redesign were authorized by the product task.
- Original intended delivery was a reviewed local squash onto main using the git-trees workflow, no push. The latest instruction to the outgoing agent is stop and hand off, not deliver now.

## Implemented and reviewed

### Tray configuration reload race

Successful settings reloads used to be followed by an unwanted supervisor restart. Pending reload delivery is now enrolled under manager synchronization before the save publishes invalidation. It carries the exact save generation and daemon incarnation. Acknowledgements are monotonic, stale completions cannot affect replacement daemons, deferred invalidations remain reachable, and failure/expiry retain bounded fallback behavior. No manager lock is held across IPC. Pending delivery TTL is 15 seconds; no new idle polling loop.

Owned changes:

- `apps/qol-tray/src/features/plugin_store/server/settings/plugin_config_handlers/{io.rs,mod.rs,notify/mod.rs}`
- `apps/qol-tray/src/plugins/config/{mod.rs,tests.rs}`
- `apps/qol-tray/src/plugins/daemon_lifecycle/mod.rs`
- `apps/qol-tray/src/plugins/manager/{autostart.rs,mod.rs,runtime.rs}`
- New `apps/qol-tray/src/plugins/manager/reload_delivery/{mod.rs,tests.rs}`
- `apps/qol-tray/tests/config_reconcile_contexts.rs`

Behavioral tests include a real barrier around publication/reconciliation, reversed completions, multiple pending saves, failure/expiry, replacement identity, full invalidation and removal. Earlier direct verification passed 1469 tests, two skipped, plus strict lint/build/doctests/dev-feature lint. Actual guest settings-save single/rapid/failure cases were exercised, with stable daemon PID/start identity and final `SHOT_CONFIG_RELOAD` evidence. See `reload-round-four-direct-verification.json` and `reload-fixed-{single,rapid,failure}.json`. The newer whole-workspace gate also passes. Do not reopen this passing slice without a concrete finding.

### Bundled native Pulse client

New `libs/audio` / workspace `qol-audio` dependency. Linux uses `pulseaudio` 0.3.1, a native Rust protocol implementation, plus `mio` for deadline-aware connect. No pactl fallback, no host libpulse requirement. Other platforms expose explicit Unsupported behavior; Linux-only dependencies stay scoped. `socket2` is a Linux test dependency for an explicitly small socket backlog.

Public API is frozen for consumers; exact fields/signatures are in the spec and source:

- `devices::{list(Direction), default_name(Direction)}` and typed device state/properties/monitor identity.
- `control::{list_cards,set_card_profile,list_sinks,list_sources,list_sink_inputs,list_source_outputs,suspend_sink,set_default_sink,server_facts}`.
- `events::Subscription::{open,recv,canceller}`, cloneable `Canceller::cancel` and typed facility/change/index events.
- Errors distinguish unsupported, unavailable server, authentication, protocol, timeout and operation failure.

Absolute monotonic deadlines cover connect, handshake and complete requests, including fragmented messages. Cookie/config reads are bounded regular-file inputs. Local client.conf, explicit PULSE_CLIENTCONFIG, config path/home/system selection, includes/drop-ins, default-server/cookie-file and env precedence are implemented with isolated fixtures. Explicit unavailable/unsupported selections fail rather than silently selecting another server. Remote endpoints and autoconnect behavior that the client cannot support are rejected.

Subscription queue capacity is 256; cancellation wakes blocked native recv/queue users and joins the reader. Fake server fixtures were corrected to actually close retained cloned sockets, emit valid modern SourceInfo wire data, saturate a deliberately small backlog and test real trickle deadlines. All 58 client tests passed with zero skips. The production configured-default setter is a low-level operation, not Sound lifetime/ownership policy.

### Consumer migrations

- Shot: Linux input/output listings and icons use typed qol-audio. Empty-on-error behavior remains, with diagnostics.
- Voice: input listing/default selection use qol-audio; PULSE_SOURCE override remains. `parec` capture remains an existing dependency. Do not claim capture engines were bundled or replaced.
- Shot ffmpeg/GStreamer capture remains unchanged.
- Bluetooth: cards/profiles, sinks/sources/streams, default adoption, suspension, server-facts and subscription transport use qol-audio. `src/platform/pactl.rs` is deleted; new `src/platform/audio.rs` owns typed selectors/request bridging. Doctor no longer requires pactl in PATH. Async callers use spawn_blocking; the event bridge is bounded and cancels/closes its receiver before joining a potentially blocked sender. Unreadable microphone-use evidence defers repair rather than authorizing it.
- Bluetooth transport migration does NOT yet implement the shared Sound default/output locks, adoption stand-down, graph-health gates or counter migration. These are explicitly still required.

Consumer paths: Shot/Voice Cargo.toml and Linux system/listen/diagnostics modules; Bluetooth Cargo.toml, platform linux/mod/windows/new audio, audio_claim Linux mod/backends/pulse_streams and hostfix Linux. Root Cargo.toml/Cargo.lock include the shared crate. `cargo metadata` and `cargo hakari generate` were run centrally after dependency changes; Hakari had no additional changes.

## Verification and latest real guest evidence

Evidence root for the following filenames:
`/tmp/code-review/sound-runtime-proof-20260920`.

`audio-round-four-check.json` is the latest complete production gate, status pass/verified, matching source fingerprint:
`48fd4a068e5d371deecb5096187b94beb8c1c8adeb07bf744059598317d68659`.

It passes the single-source guard, UI tests, release-script tests, formatting, workspace build, strict Clippy, nextest and doctests. The repository-owned command excludes `keyremap` and enables `qol-voice/local-stt` and `qol-voice/sherpa-stt`; do not call it a keyremap or cross-platform pass. Exact argv/exit status are in the JSON. The gate used `qol check --base <baseline> --report <file>` with repeated `--format-owned <path>` for the changed/new Rust files. No production changes followed this pass; later edits were the external proof leaf and spec. Earlier wrapper errors sometimes said descendants remained; do not confuse old failed reports with the current green one.

Latest guest run:

- Aggregate: `sound-native-consumers-20260920`.
- Lane: `linux-mint-cinnamon-18d7053c4a711349-225f85-0`.
- Bundle SHA: `4e5efd245c04cb3bb0192cc6a49a99d7cfaa79b89af7c734c45ce4bb5461e4d3`.
- Image: Mint 22.3 / Cinnamon 6.6.7 qol-2, image SHA `7af02ffc7f2f5fb97ce01aa1cf9222c61e5e079bf48298afdbd5a855e0c1a973`.
- Headless, network none, no host workspace mount or USB device passthrough. Host-built debug artifacts carried by read-only payload.
- Report: `native-consumers-round-five.json` and sibling transcript.
- Leaf: `native_consumers.py`, SHA `dc1431f98fd07284ff4ad8a899e093ca896b195551545e49f03a286616916453`, still unchanged after the interrupted sixth round.
- Runtime 3.862 seconds; 54-second limit, 22-second cleanup reserve.

Observed facts: real Shot sink/source queries and Voice source query all returned HTTP 200 with the correct fixture name/description/icon; Voice `audio devices --json` passed every assertion. pactl, pw-dump, pw-cli and pw-link each raised FileNotFoundError in the actual restricted consumer PATH. Both daemons had stable before/after PID/start identity and matching bundle executable digests/hardlink identity. Unit restart/restore, original PATH/configuration restoration, fixture unload and allowlist removal passed. The guest was subsequently stopped; aggregate report has stopped/complete/verified-cleanup and payload removed. No running guests remained.

**The proof's overall verdict is still false. Do not silently relabel it as passing.** Four failed conditions come from two known assertion defects below, not failed consumer responses. The next agent must fix those and rerun. `proof-round-five-review.json` contains the isolated wrapper counterexample. Keep the failed report immutable.

## Immediate unfinished correction and interrupted lane

Sixth proof round was interrupted by a provider error before editing. The receipt is markerless and the report shows reads only. The source hash above and both defective functions were rechecked when writing this handoff. No retry was submitted after the user's stop instruction.

- Lane key/title: `sound-proof-impl`, tool pi, configured model deepseek-flash.
- Last token: `v1:kitty:k10304_f0001a.45:2271675`; it is closed, not reusable as a live target.
- Marker: `QOL_BRIDGE_DONE_e472d428593a1cd1d0b3`.
- Receipt directory: `/home/kmrh47/.local/share/qol-tray/sessions/lanes/rounds/346fbad14624654bf1aeb79e63c1f5bccdd3e2092338cac37866b17326f09931/`.
- Prior completed fifth-round report: `/home/kmrh47/.local/share/qol-tray/sessions/lanes/rounds/ee15ed82c34728456728dc2ef1372102ac8348a08e162de3c695091108566baf/report.md`.

Exact remaining edits, solely in `/tmp/code-review/sound-runtime-proof-20260920/native_consumers.py`:

1. `bound_query` around line 963 stores all of wait_for_query's `{entry, complete, elapsed_seconds}` wrapper as `entry`. The three assertion functions expect inner `{status, payload, ...}`. Unwrap once at that boundary, preserve completion/timing separately and require completion in the verdict. Do not alter production responses or weaken assertions.
2. `authoritative_inventory` around line 703 expects `monitor_of_sink` fields absent from this guest's pactl JSON. Actual fixture sink.monitor_source is the monitor name; fixture monitor.monitor_source is the sink name and properties.device.class is `monitor`; the remapped source.monitor_source is empty and device.class is `filter`. Assert the mutual sink/monitor relationship and independently non-monitor source using the actual schema. Do not settle for a suffix or remove monitor verification.

The latest spec includes the exact narrow lane brief. The 1315-line proof has already had several correction rounds; avoid broadening or redesigning the now-working unit/path/identity/cleanup mechanism. There may be further review findings; completion-marker prose is never acceptance. Syntax and pure-function counterexamples can be checked centrally without running the leaf's main on the host.

## Important backend experiments: retain both successes and failures

These experiments used guest CLI instrumentation, not the new bundled PipeWire client. They do not establish a finished Sound switch.

- `endpoint-audio-main.json`: client-owned, non-lingering virtual sink/links disappear on clean release and SIGKILL; a continuing player returns to A; configured/default history remains unchanged; future fallback C works.
- `endpoint-audio-monitor.json`: target monitor captured actual stereo PCM with dominant 440 Hz left and 660 Hz right, active owned links and clean release. This is not human audibility or earbud evidence.
- `endpoint-audio-input.json`: **the candidate is unsuitable as an output-only backend**. Its high-priority virtual sink takes over default input both for automatic monitor fallback and for an explicitly configured non-monitor source. Release restores input/history, but temporary takeover is still a product failure. Do not enable this backend on the strength of other passing dimensions.
- `default-fixed-all.json`: directly overriding effective default metadata honored the choice without changing configured history, but did not revert on owner death. Not sufficient for the required lifetime contract.
- External configured output selection can also be overridden by the virtual sink's high priority until owner exit; user-choice precedence needs a real solution.

Verified upstream WirePlumber 0.4.17 `modules/module-default-nodes.c` includes sinks with output ports among source candidates and uses shared priority.session scoring. A very high virtual sink priority can outrank even the configured input bonus. Do not repeat that disproven design under a new label.

No reproduced repair wedge or live-earbud ladder-order test exists. Mutating repair rungs remain gated; relink stays disabled.

## Full product work still required

1. Finish the native-consumer guest proof above, then continue the original product, not another general existence scout.
2. Resolve and implement a bundled native PipeWire graph/endpoint capability. Prior research found `pipewire-native` 0.1.4 has client-side SPA/config dynamic dependencies, while the usual pipewire crate links host libpipewire. Neither meets the self-contained requirement merely by declaring a Rust dependency. Resolve packaging or implement the needed protocol capability. A graph-Unavailable stub is not completed delivery.
3. Implement Sound headless commands and owner/session/release state, then daemon/contracts/settings with actual versus saved status. Existing native contract surfaces are the intended UI route; no new framework or cross-plugin communication channel.
4. Implement stable device identity/kind, complete configured-versus-automatic/default-history snapshots, standalone switch snapshot writes and capability refusal before a switch that cannot be undone. Meet Portable owner-death behavior without assuming a later process start; implement Resident durable choices and explicit `--keep` semantics separately.
5. Use real data-directory file locks, never unlink a lock inode: global default-output lock plus per-output lock in a fixed acquisition order. Durable repair/choice records must fail closed when unreadable/corrupt.
6. Integrate both Bluetooth reclaim paths with those locks and make Sound stand down for Bluetooth adoption of a reconnecting headset. Add graph-health NotDefault/CannotTell handling and shared counter migration; unreadable safety evidence must never permit repair.
7. Add Resident release on residency disable and before uninstall. Public give-back command is `plugin-sound give-back [--abandon]`; doctor --fix was removed by the accepted design.
8. Finish diagnosis, interruption-safe repair/scheduler behavior and the design's reproduced-wedge/live-earbud gates before enabling the appropriate mutating rungs. Relink is removed from the default ladder because it cannot safely be interrupted and no next start is guaranteed. No last-rung cross-plugin host_exec action.
9. Review and validate the complete integrated diff, then the authorized local delivery workflow. Do not report the current client/tray subset as full Sound implementation.

## Reuse research and session history

Useful research artifacts under `/home/kmrh47/.local/share/qol-tray/sessions/groups/`:

- `sound-runtime-fixes-scout/rounds/1/combined.md`
- `sound-full-implementation-scout/rounds/1/combined.md`
- `set-2-c91839ca/rounds/{1,2,3}/combined.md`
- `set-3-899eccfe/rounds/{1,2,3,4}/combined.md`

Round-three audio/proof stalled markers were intentional compaction interruptions, not discarded work. Audio and proof were resumed with native `/compact`, then continued successfully; all resulting edits are retained. Saved pi histories were audio `01a0be61-f275-7575-9233-477fa34b62af`, consumers `01a0be61-f492-77f4-9cae-6482dc30e070`, proof `01a0be25-8fd9-7263-8d60-85be9e47b503`. Prefer stable lane keys and public session discovery; do not invent or reuse closed live tokens.

The installed sessions CLI/API lags the skill in some details. A fresh `qol sessions mcp` process with cwd set to the worktree successfully supports `session_spawn` arguments shaped `{tool: "pi", cwd: <worktree>, lanes: [{key, title, task}]}` and returns task_submitted/background/autoclose, token, marker and resume information. It sometimes auto-labels lane sets as grouped research even for implementation; those labels are transport behavior, not task scope. A single task using top-level key/background failed on this installed interface; the lanes-array form worked. The registered long-lived spawn MCP had stale cwd; do not silently launch in main.

If reproducing that transport, use a properly argv-quoted Python or Node JSON-RPC client: initialize, notifications/initialized, tools/call session_spawn. Keep MCP stdin alive until watcher completion so wakes are delivered, then close it. Verify title with sessions_list, end the architect turn after delivery and resume on the watcher receipt. No session polling or workflow redesign. Model/role overrides were omitted because current configured defaults are authoritative and extra role metadata had previously refused this host.

## Guest commands and evidence workflow

Use the existing argv-safe workflow leaf driver rather than inventing a QMP transport:

```text
qol env up linux/mint-cinnamon --dev-worktree <absolute-worktree> --run-id <new-unique-run-id>
node /tmp/code-review/sound-runtime-proof-20260920/run-guest-proof.mjs <lane-id> /tmp/code-review/sound-runtime-proof-20260920/native_consumers.py <new-report.json> <actual-bundle-manifest-sha256>
qol env down <lane-id>
```

Read the exact lane and bundle digest from `<worktree>/target/qol-env/<new-run-id>/report.json`. Always pass the freshly observed bundle SHA as the optional scenario argument; the leaf now requires it. The old payload has been cleaned up. The node driver records leaf SHA, command, timings, output and structured evidence; its successful exit only means transport/JSON success, not `evidence.verdict.pass`. Inspect the verdict and cleanup explicitly.

Guest-control commands must be serialized because they share an identity lease. `qol env down` with the aggregate id previously failed selection; the exact lane id worked. Check the aggregate teardown report and `qol env runs` afterwards. A generic guest without the artifact payload did not provide the needed image identity; use --dev-worktree. Never print the full command embedded in a failed report because it contains the entire base64 leaf; select concise report fields instead.

All external proof scripts and evidence live under /tmp and should be preserved if the machine may be cleaned or rebooted. This handoff and the implementation spec live in the worktree. No code was changed while producing this handoff.
