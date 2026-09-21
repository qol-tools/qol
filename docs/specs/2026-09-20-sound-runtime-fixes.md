# Sound implementation: bundled audio client and runtime reliability

## Outcome and active slice

The user authorizes implementation of the Sound plugin, the bundled audio client, migrations of the existing audio consumers, and the remaining runtime fixes from the reviewed design.
The design sheet is the product specification.
This document records implementation ownership, interfaces and proof gates; it does not reopen the product scope.
The production tray defect is that a successful settings reload is followed by a supervisor restart.
The failed WirePlumber experiment establishes that selecting the previous sink name is not a valid way to put the host default back.
Sound does not put anything back: the saved `output.device` value is the only record of the user's choice, and qol never reverts it on exit, crash, residency disable or uninstall.

Evidence: `/tmp/code-review/sound-runtime-proof-20260920/review.md`, `reload-save.json`, `audio-history.json`, and `report.json`. Source baseline is recorded by those artifacts. The accepted design snapshot is `/tmp/code-review/sound-design-fixes-after-v33/after/arch.mjs`.

The full delivery includes `plugins/sound`, `libs/audio`, Shot/Voice/Bluetooth consumer migration, the Bluetooth adoption safety gate, and the tray fix.
No scope question remains pending.
New host services, host configuration changes and sessions-tooling redesign are not implied by this authorization.

## Sound choice rule

A host setting the user changes through qol is the user's change, not a qol mutation.
Choosing the default audio output or the volume runs the host's own mechanism at the user's direction.
Sound records no lifetime for it, snapshots nothing and never reverts it on exit, crash, residency disable or uninstall.
Sound has no give-back, no restore, no ownership record, no snapshot, no lifetime, no keep, no abandon and no lock file.
A switch resolves the requested output to one device, sets it as the host default, reads the effective default back and reports success or the real error.
Nothing is written to disk by Sound except the host's own default.
Choosing System Default (`SYSTEM_DEFAULT = "default"`) changes nothing on the host; it means Sound does not pick an output.
The saved `output.device` value is the only record of the user's choice.
On daemon start Sound applies a saved non-default choice once when that output is connected; it never switches automatically when a saved output reappears later.
The tray never asks a plugin to give anything back: no give-back on residency disable and none before uninstall.
Bluetooth becomes the default output only when no other usable output exists: the effective default is absent, is a null or dummy sink such as `auto_null`, or names a sink that no longer exists.
If any other real output exists, Bluetooth leaves the default alone and records nothing to retry.
Reclaim-on-play was removed on purpose; the manual `reclaim_device` action stays without any lock.

## Research synthesis and implementation sequence

The completed research is retained at `/home/kmrh47/.local/share/qol-tray/sessions/groups/sound-full-implementation-scout/rounds/1/combined.md`. Its repeated observations that Sound is new are not implementation findings and require no further investigation. Use its concrete callsite inventories, existing shared API evidence and upstream references. The accepted design snapshot remains authoritative when a scout proposal conflicts with it.

1. Active implementation round: fix tray reload delivery and author the guest proof leaves below. These are implementation tasks, not more scouting. The design explicitly requires the tray race tests and actual guest save proof before Sound code. The architect runs those gates centrally after reviewing both lanes.
2. Implement `qol-audio` and its bundled client, then migrate the listed consumers atomically before any release. The Pulse control client candidate is the pure Rust `pulseaudio` crate; it covers the existing control operations through high-level and protocol APIs. The PipeWire graph/endpoint client remains a packaging decision: the proposed `pipewire-native` crate has client-side SPA/config dependencies, and the ordinary `pipewire` crate links a host library. Neither may be called self-contained without resolving those dependencies. No workaround may quietly introduce a required host CLI or library. Freeze the concrete shared API and exact edit paths in this spec before dispatching those lanes.
3. Implement Sound headless commands, doctor and meaningful behavioral tests against that API.
Then implement the hosted daemon and contract-rendered settings, including saved-versus-applied status.
A switch resolves the requested output to one device, sets it as the host default, reads the effective default back and reports success or the real error.
4. Implement read-only graph diagnosis and scheduling tests.
Enable a repair only after the design's interruption and reproduced-wedge gates pass; validate ladder ordering with the integrated guest earbud run.
Relink remains disabled.
An unavailable backend operation must give an explicit reason; that is not permission to present a read-only scaffold as the completed full implementation.

The native client is a shared capability, not a cross-plugin broker. Shot listing lives in `plugins/shot`, despite its public id being `qol-shot`. The consumer inventory includes Voice source listing/default resolution and every Bluetooth Pulse control/subscription call. Capture engines such as ffmpeg and Voice capture are separate existing capabilities; the final report must state exactly which operations were migrated rather than claiming all audio capture dependencies disappeared.

The virtual-output proposal is an experiment only.
A client-owned node and links without linger might let WirePlumber select and remove the temporary output without modifying configured/history policy.
It is not yet proven, and the scout's claim that it eliminates the need for any host-default change is rejected: a process-scoped endpoint does not by itself change the host default at the user's direction.
External manual choice precedence, volume/mute, stream pinning and host UI behavior also require evidence.
The final backend must set the host default through the host's own mechanism, not redefine that rule.

The design's A/B and N1/N2 presentation alternatives remain optional; use the existing supported contract card and mandatory status fields first. A second declared `output_status` query is a routine implementation detail required by those fields. Reconnecting a missing saved device does not silently introduce automatic adoption. No Sound plugin UI framework or new cross-plugin channel is authorized or needed.

## Bundled client surface after the PR 27 review

`devices::default_name(Direction)` was removed.
Its platform twins are gone with it.
The single owner of default resolution is `default_output::effective(direction)`.
Voice's `default_source_name`, `examples/devices.rs` and `examples/inventory.rs` call `qol_audio::default_output::effective`.
`control` is Linux only.
`libs/audio/src/lib.rs` declares it under `#[cfg(target_os = "linux")]` and its unsupported stubs are deleted.
This supersedes the `devices::default_name` entry in the earlier frozen interface block.

## PipeWire graph capability: not implemented, still open

No `qol_audio::graph` module exists in the tree.
No `libs/audio/src/platform/linux/graph/` directory exists.
No POD codec, framing, handshake, registry, metadata or node and port info decoder is delivered.
`libs/audio` carries no PipeWire dependency at build or run time.
The chosen route and the rejected crates remain decisions in the graph research section below.
The metadata write permission test and the version 4 hello handshake against a live server remain unproven.
Item 2 of the handoff's remaining-work list owns this work, and both documents agree on the open status.

## Native consumer guest proof passes (sixth round)

The sixth round corrected the two assertion defects listed under the third review and reran on a fresh offline headless artifact-backed Mint guest.
Report `/tmp/code-review/sound-runtime-proof-20260920/native-consumers-round-six.json` binds leaf SHA `e1ac3810e5e1313a4f4bda6c4fe371207fa0a3747bd0c0e4d0bc3f0b359a0506` and bundle manifest SHA `4f236c045c7fb22bec79ba3574a72de48d1ef788bf93c0c47a19cebf3b7eb47b`, run id `sound-native-consumers-20260920-r6`, lane `linux-mint-cinnamon-18d705df8b96dd77-241c9f-0`.
All 26 pass conditions are true, `verdict.pass` is true, there are no failed conditions and no fatal error, and cleanup reported ok.
Shot `audio_sinks`, Shot `audio_sources` and the Voice `audio_sources` tray query each returned HTTP 200 with every per-section check true, and each now records `query_complete` true with its own elapsed time under the new `bound_queries_complete` condition.
The corrected inventory asserts the real guest schema: the fixture sink's `monitor_source` is the monitor name, the monitor record points back at the sink with `device.class` monitor, and the remapped fixture source has an empty `monitor_source` with `device.class` filter.
All four forbidden tools were genuinely absent from the restricted PATH while really present on the guest, and the manifest comparison matched the supplied expected identity.
The fifth-round report stays immutable as the record of the failing state; the guest was torn down through its own lane id and `qol env runs` reports nothing running.

Both corrected pure functions were also checked centrally on the host against counterexamples without running the leaf's main: a remapped source that names a sink in `monitor_source`, one whose `device.class` is monitor, a broken mutual sink/monitor relationship, a monitor record missing `device.class`, and a source record with no `properties` at all.
Each behaves as intended, so the monitor verification was strengthened rather than loosened.

Next in the product sequence is the bundled native PipeWire graph and endpoint capability, item 2 of the handoff's remaining work.

## PipeWire graph capability: research settled and route chosen

Research evidence is in the `sound-pipewire-packaging-scout` lane report. The decisions below are the architect's and are binding for the next implementation rounds.

What the already bundled Pulse client can and cannot carry. `pipewire-pulse` has no link or port object and no link or port command, and its object model is sink, source, sink-input, source-output, client, module, card, sample, server. So the Pulse path already carries: reading the effective default through `GetServerInfo`, which `pipewire-pulse` fills from `default.audio.sink`; writing the configured default, because `SET_DEFAULT_SINK` writes `default.configured.audio.sink`; enumerating audio nodes as sinks and sources; and coarse change notification through `SUBSCRIBE`. It cannot carry: writing the effective `default.audio.sink` or `default.audio.source` key, any port or link enumeration, link creation or destruction, detecting that a node lost its links, or reading WirePlumber's default history, which is module-internal `WpState` and is not on any protocol.

Rejected routes and why.
`pipewire-native` 0.1.4 is rejected as a dependency. Its `Support::load_spa_handle` dlopens `support/libspa-support.so` from `SPA_PLUGIN_DIR`, and `init` and `MainLoop::new` each `expect` factories out of that shared object, so the C plugin is not avoidable by configuration. Shipping it would mean embedding a per-architecture C blob and extracting it at startup, which defeats the self-contained requirement rather than satisfying it. The crate also decodes link state wrongly: its `LinkChangeMask` defines only `PROPS = 1 << 0` while the server sends `STATE`, `FORMAT`, `PROPS`, and its demarshal unwraps a `TryFrom` on a 0-based enum where the C values for `ERROR` and `UNLINKED` are negative, which is exactly the wedge-detection path we need. Its config load and `spa-json-dump` subprocess are separately avoidable through `config.name = "null"`, but that does not rescue the plugin dependency.
`libspa` 0.8.0 is rejected for POD reuse: its manifest declares `[package.metadata.system-deps.libspa] version = "0.2"` and depends on `spa_sys`, so it links host C. Verified directly from the published crate.
`pipewire-native-spa` 0.1.4 holds a genuinely pure-Rust `src/pod` module, but its `build.rs` unconditionally `cc`-compiles `src/support/ffi/log.c` and `src/support/ffi/system.c` and probes `pkg-config`, so depending on it drags a C toolchain and a pkg-config probe into every platform build for the sake of a serializer. Not taken.
Statically vendoring libpipewire, and the Pulse `LOAD_MODULE module-null-sink` plus `module-loopback` endpoint, are both rejected: the first contradicts the bundled Rust client intent, and the second is server-owned and needs host `libpipewire-module-*.so` files.

Chosen route. Implement the needed subset of the PipeWire native protocol as a new module inside the existing `qol-audio` crate, pure Rust, no C at build or run time, reusing the crate's existing `mio` deadline discipline. Scope the first slice to what Sound actually needs now and nothing more: transport and the 16-byte header, the `core.hello` handshake at version 4 with the `sync`/`done` barrier, `core.get_registry` with the `global` and `global_remove` events, `registry.bind`, the metadata interface with `set_property` and the `property` event, node and port `info` demarshal, and a POD codec covering struct, object, int, long, string, id, bytes and array. Link creation and destruction stay out of this slice, because relink is already removed from the default ladder and cannot be safely interrupted; link `info` decoding, including the negative `UNLINKED` and `ERROR` states, comes in with the graph-health work.

Blockers that still need live evidence, in the order they gate the work. Metadata write permission: `metadata.set_property` requires `W` on the metadata global and `M` on the subject, and the access module can restrict it, so a guest test must write `default.audio.sink` from our own client and read it back through `GetServerInfo`. The version 4 `hello` handshake has not been executed against PipeWire 1.0.5 from Rust in this research. Default history remains invisible on every protocol, and the switch does not read it: Sound resolves the requested output, sets the host default and reads the effective default back.

## Native client third review and proof correction

The fifth-round proof ran centrally in a fresh offline headless artifact-backed Mint guest. Report `native-consumers-round-five.json` binds leaf SHA `dc1431f98fd07284ff4ad8a899e093ca896b195551545e49f03a286616916453` and bundle SHA `4e5efd245c04cb3bb0192cc6a49a99d7cfaa79b89af7c734c45ce4bb5461e4d3`. Real Shot sinks/sources and Voice sources queries returned the expected nonempty fixture records; Voice CLI passed every assertion. All four forbidden tools actually raise FileNotFoundError under the restricted PATH. Both query daemons had stable before/after PID/start identity, restricted environment and matching manifest digest/hardlink identity. Unit configuration/PATH restoration, fixture unload and temporary removal all passed. Runtime was 3.862 seconds. The guest was then stopped through its owned lane. This is substantive positive native-client evidence, but the leaf's overall verdict correctly remains failed until the two assertion defects below are fixed and rerun.

Sixth-round proof correction owns only `native_consumers.py`. Do not redesign the working mechanism or touch passed production code. Two concrete fixes:

1. `bound_query` currently places the entire wait_for_query result in its entry field. That is `{entry: {status, payload, ...}, complete, elapsed_seconds}`, while shot_sink_checks, shot_source_checks and voice_query_checks expect the inner `{status, payload, ...}`. Unwrap exactly once at the boundary, retain completion/elapsed evidence separately, and require completion in the measured verdict. `proof-round-five-review.json` demonstrates that all sink checks fail on the wrapper but pass on its actual entry. The three returned guest payloads in `native-consumers-round-five.json` are already correct; do not weaken assertions or change production responses.
2. Guest pactl JSON represents the monitor relationship as `monitor_source`, not `monitor_of_sink`/`monitor_of_sink_name`: fixture sink.monitor_source is the full monitor name; fixture monitor.monitor_source is the sink name and its properties.device.class is monitor; the remapped non-monitor source.monitor_source is empty and device.class is filter. Verify this explicit mutual sink/monitor relationship and the independently non-monitor remapped source using the actual recorded schema. Do not merely accept a name suffix or delete monitor-presence verification. If supporting other schema variants, preserve explicit identity checks and keep the known guest case unambiguous.

Read the saved JSON and the existing owned source only. Do not execute scripts/tests/guests or edit evidence; the architect will run the corrected leaf on a fresh guest and keep this failed report unmodified. Report paths/lines and any conscious deviation only.

Fourth-round central verification is green: `audio-round-four-check.json` records successful workspace build, strict Clippy, nextest, doctests, formatter, single-source guard, UI and release-script checks, with matching source fingerprint `48fd4a068e5d371deecb5096187b94beb8c1c8adeb07bf744059598317d68659`. The repository's workspace gate excludes keyremap and enables Voice local-stt and sherpa-stt; do not broaden that result into an unsupported-platform claim. No production corrections remain from this round. Guest execution still waits for the proof-only corrections below.

The architect personally reviewed all 1133 lines of the fourth-round leaf and compiled it without execution. `audio-round-four-proof-review.json` records isolated pure-function counterexamples: a restricted parent lets an unobserved daemon pass; one restricted daemon lets a mixed restricted/unrestricted set pass; identical stable ExecStart argv with changed PID/timestamps fails the raw equality used in cleanup. Correct only `native_consumers.py`:

1. Compare stable unit configuration for restoration, including parsed executable/argv, WorkingDirectory and QOL_DEV_ARTIFACT_ROOT. `systemctl show ExecStart` includes runtime PID/start/stop/status fields that necessarily differ after restart. Do not compare that entire raw string or drop the configuration identity check. Verify exact cgroup membership (the parsed cgroup path equals the owned path or is its child), not substring matching.
2. Delete the unobserved-consumer fallback. The real route is `apps/qol-tray/src/features/plugin_store/server/plugin_handlers.rs::query_plugin_handler` -> `plugins/action_executor/mod.rs::dispatch_query_with_input`, which exclusively dispatches to the daemon and may ensure/retry its startup. It has no one-shot query subprocess fallback. Require separately observed Shot and Voice daemons in the verified unit, each with restricted PATH and expected executable digest. Require all candidate processes restricted, not restricted_count > 0. Bind each query to matching before/after PID/start identity or fail if it changed, so a daemon merely observed after the query is insufficient. The separate Voice CLI already has explicit env and remains separate evidence.
3. Resolve manifest identities through the actual payload install mapping. `flows/envs/linux-mint-cinnamon/qol-sandbox-payload::publish_dev` hardlinks `current/plugins/...` into `/home/qol/.config/qol-tray/plugins/...`; real daemon exe paths can therefore be outside artifact_root. Map each known consumer to its exact manifest key and verify the actual process executable bytes against that key, additionally checking the installer mapping or same-file identity. Never require an incidental artifact_root prefix, and never accept arbitrary basename matches. A missing/unparsed manifest or missing consumer digest fails the proof. The central runner will provide the exact expected manifest SHA through its existing optional scenario argument; require the expected value and a match for this artifact-backed scenario. This is not permission to change the driver or guess a SHA.
4. Track allowlist/drop-in creation before operations that can partly succeed and then throw. Remove an owned partial drop-in even if write_text did not reach written=True. Independent cleanup stages must continue after an unexpected restoration exception, preserving the primary failure and each cleanup result; the present outer exception handler skips remaining fixture cleanup. Retain the allowlist only when needed by an unrestored unit and report failure. Keep the existing work deadline and cleanup reserve.
5. Remove the redundant alternate Voice argv retry; the supported command is audio devices --json. Check authoritative descriptions rather than treating missing expected labels as success. Keep scope narrow: no new framework, no speculative mechanism, no new runtime work or changes to passed production code. Report exact edits and remaining limitations only.

The compacted audio and proof continuations completed with their history retained. The old third-round audio/proof stalled markers describe the intentional interruptions, not lost implementations. Bluetooth's uninterrupted migration also completed. The architect reviewed the actual changes and ran the central gate: `audio-round-three-check.json` fails compiling Bluetooth's tests at `src/platform/linux.rs:3257,3261` because active_card_profile now returns Option<String>, whereas expectations remain Option<&str>. Correct these typed expectations in that owned file without changing the production API or removing either assertion. Retain the native migration. Any further compiler/lint findings recorded alongside this section belong to the same narrow correction.

The focused central `cargo nextest run --locked -p qol-audio --no-fail-fast` now passes all 58 tests, zero skips; bounded backlog, trickle, modern source mapping, subscription loss and cancellation all execute successfully. The central strict client lint still fails at `environment.rs:361`, manual_pattern_char_comparison. Audio correction owns only that file and replaces the manual two-character predicate with the standard character-array pattern, preserving semantics and adding no allow attribute. Evidence: `audio-round-three-focused-tests.log` and `audio-round-three-clippy.log`. No other audio changes are requested in this correction.

The new `native_consumers.py` is not approved for execution. Its current stub executables are discoverable tools, its manual tray restart races the dev supervisor, and its cleanup pass flag ignores nested failures. Proof correction owns only `/tmp/code-review/sound-runtime-proof-20260920/native_consumers.py`; production paths and existing evidence are read-only.

1. Use an actually restricted consumer PATH that cannot resolve pactl, pw-dump, pw-cli or pw-link. A temporary directory of symlinks to the other required executable names is acceptable; omit all four tools rather than substituting executables that return 127. Keep instrumentation at explicitly addressed absolute paths outside that PATH. Do not rename installed guest tools or modify system-wide/manager environment.
2. Restart through the existing guest dev unit owner, never kill/relaunch a tray while its supervisor remains running. `tools/qol-cli/src/commands/env/dev_session.rs::launch_dev_unit` launches `qol-dev-{run_id}` with systemd-run --user, bundle working directory, QOL_DEV_ARTIFACT_ROOT and bundle/bin/qol dev. Verify the exact unit against the guest run identity, ExecStart, working directory and artifact root before changing it. A uniquely named temporary runtime drop-in for that verified unit, daemon-reload and bounded restart can carry PATH. Preserve prior configuration and restore only the owned drop-in in finally, then restart the same unit with its original environment. Never stop arbitrary processes selected by executable basename or change unrelated units. If the owner cannot be verified or controlled, return explicit failed evidence without substituting a manual process launcher.
3. Bind HTTP discovery and consumer evidence to the verified unit and actual executable identities. Require Shot and Voice evidence separately, including the real processes executing the queried paths; an arbitrary nonempty subset of daemon names cannot pass. A query that executes in a one-shot subprocess must prove the inherited restricted environment through its verified parent and actual route. Record PID/start identity and executable digest, not secrets or entire environments. Voice's actual `audio devices --json` invocation also needs the restricted environment. Keep all source/name/icon/monitor assertions and make expected descriptions agree with authoritative fixture inventory.
4. Remove the previous bundle's hardcoded expected digest. Read/hash the current bundle manifest and actual consumer executables, record the observed identity, and bind the guest report to the artifact identity supplied by the central runner. Do not manufacture an expected value from a stale report or claim comparison when none occurred.
5. Derive every cleanup result from actual successful restoration, fixture unload, owned temporary removal and restored unit readiness/environment. Accumulate nested errors. Any fatal work error, cleanup error, timed-out restoration or false removal result must make the overall proof fail. Never set cleanup_complete unconditionally. Cleanup should continue best effort across failures while retaining each outcome and preserving the primary error.
6. Enforce the work deadline at every stage and before each subprocess/HTTP attempt. A depleted budget must stop work and enter cleanup; do not clamp a negative remaining budget to a positive timeout and keep issuing operations. Reserve enough time for both unit restarts and cleanup inside the existing transport limit. Trim optional Bluetooth or redundant control queries first; if necessary implement an explicit bounded scenario rather than overfill a 54-second leaf. Do not silently increase transport budget or change the driver.

The architect will centrally review the corrected source, compile-check the Python leaf and execute it only in a fresh disposable artifact-backed guest after the code gate passes. No host runtime actions, no guest execution by lanes, and no changes to the prior endpoint experiments. The confirmed default-input takeover remains an unresolved Sound backend issue, independently of the native-consumer proof.

## Native client second review and Bluetooth migration round

The architect reviewed the second audio correction. `audio-round-two-check.json` passes the workspace build (including Voice STT features), formatter, single-source guard, UI and release-script checks. Strict Clippy stops at `libs/audio/src/platform/linux/tests.rs:1037`, `type_complexity`. The focused audio/consumer run found four failures and two hung subscription cases; the architect interrupted its owned nextest process after the two cases exceeded sixty seconds. Transcript: `audio-round-two-focused-tests.log`; 34 passed, six failed including the two interrupted cases, one skipped, 179 not run. A separate consumer run then passed all 178 Shot/Voice tests, one skipped (`audio-round-two-consumer-tests.log`). None of this is a passing full client gate.

Audio correction owns `libs/audio/Cargo.toml`, `libs/audio/src/**`, `libs/audio/tests/**`, `libs/audio/examples/**` only; no root manifest, lockfile, vendor or consumer edits. Preserve every frozen public API, including `events::Subscription::{open,recv,canceller}` and cloneable `events::Canceller::cancel`.

1. Resolve Clippy's complex fixture-case type through a named fixture type or simpler table, without suppressing lint.
2. Fix the fake server's actual disconnect. `Fixture.client` retains a cloned socket after `serve` returns, so `Step::Disconnect` leaves the peer open: request tests time out and subscription loss/partial-frame tests hang forever. Close/shutdown the tracked connection on every server exit. Make test receive assertions and teardown bounded even on production failure; no leaked detached receiver thread. Keep unused-fixture teardown coverage.
3. The full-backlog fixture assumes 512 connects fill the kernel backlog, which is false here. Bind a small explicit listen backlog with a safe existing socket API (a narrow target dependency if necessary), prove saturation, then exercise the client's absolute deadline. Do not weaken the deadline or skip the case on this host.
4. The source mapping fixture fails `Protocol("expected U32, got U8")`. The pinned upstream SourceInfo encoder omits availability_group and port_type for protocol >=34, although its decoder expects them; SinkInfo's encoder includes them. Correct the fixture to emit independently valid native source wire data including those fields. Keep modern protocol, populated port and monitor/non-monitor coverage; do not alter a correct production decoder to accommodate a broken test encoder or hide coverage by dropping all ports. Inspect a production defect separately if the real guest exposes one.
5. The current slow-fragment test sends one prefix then stalls longer than the per-read timeout. That passes with the old per-read implementation too. Send several valid fragments each sooner than the per-read limit but spanning more than the operation deadline, and assert a bounded whole-request timeout. Keep the silent-server case.
6. Complete local connection configuration support. `environment.rs` currently ignores client.conf entirely. Honor explicit PULSE_CLIENTCONFIG and user/system Pulse client configuration selection, relevant default-server and cookie-file settings, and env overrides. Do not silently connect to a different server when selected configuration is unreadable/malformed or names an unsupported endpoint. Parse includes/drop-ins where applicable with bounded size/depth, or explicitly reject an unsupported directive that would affect connection selection. Never autospawn a server or write config/cookies. Retain local-only endpoint policy and typed explicit remote rejection. Cookie/config input must be bounded regular-file reads, not a FIFO that can hang before socket deadlines. Isolated fixtures must cover precedence, explicit failure, includes/cycle handling, absent optional defaults and no host-secret access. Primary reference: PulseAudio v17 `src/pulse/client-conf.c`, https://raw.githubusercontent.com/pulseaudio/pulseaudio/v17.0/src/pulse/client-conf.c (configuration loaded before environment; default-server and cookie-file are connection inputs). Use its referenced core config helpers for exact file selection; retain the deliberate fail-closed behavior for explicitly selected unavailable cookies.

Bluetooth migration proceeds against the now-frozen typed API concurrently. This lane owns exactly `plugins/bluetooth/Cargo.toml`, `plugins/bluetooth/src/platform/linux.rs`, `plugins/bluetooth/src/platform/mod.rs`, `plugins/bluetooth/src/platform/windows.rs`, `plugins/bluetooth/src/platform/pactl.rs` (remove), new `plugins/bluetooth/src/platform/audio.rs` if shared typed selectors merit it, `plugins/bluetooth/src/audio_claim/platform/linux/mod.rs`, `plugins/bluetooth/src/audio_claim/platform/linux/backends/pulse_streams.rs`, and `plugins/bluetooth/src/hostfix/platform/linux.rs`. The fixed-string pactl/parse_short_sinks/running_sink_except searches locate all Bluetooth hits in this set, including the Windows diagnostic null field. No Shot/Voice or shared-client edits.

Use Linux-only qol-audio workspace dependency. Replace all listed Pulse CLI calls and text parsers with `control::{list_cards,set_card_profile,list_sinks,list_sources,list_sink_inputs,list_source_outputs,suspend_sink,set_default_sink,server_facts}` and typed devices/events. Preserve profile selection, address matching, running-other-output protection, playback-start bookkeeping, source-in-use observations and existing retry bounds. Keep server failure distinguishable from an empty list and never turn unreadable safety evidence into permission for a new mutation. Preserve useful error context and trace events. Doctor must report the bundled client rather than require pactl/install utilities; preserve existing check IDs. The hostfix server-name observation consumes server_facts, leaving unrelated hostfix actions unchanged.

The synchronous client must not block Tokio's executor: use bounded spawn_blocking for requests and a cancellable bounded bridge for subscription events. An aborted async watcher must cancel native recv, release any blocked queue sender and reap its reader; closing a channel only after joining its blocked sender is a deadlock. Idle subscriptions block on events, not sleep-poll. Retain the existing reconnect delay only after actual connection loss. Convert parser-only tests to typed semantic cases for corked inputs, dangling sink indices, monitor/source identity, active profile and running-other-output blocking. Add a meaningful cancellation bridge fixture if introducing that bridge. No host audio, BlueZ calls or fixtures against real user sockets in tests.

This round migrates the transport only.
Bluetooth's adoption gate, health/counter migration and interruption gating remain mandatory subsequent work alongside the corresponding shared capability; this partial migration is not independently shippable.
Do not invent Sound state in Bluetooth.

Proof lane owns new `/tmp/code-review/sound-runtime-proof-20260920/native_consumers.py` only. Author a bounded guest leaf using the existing run-guest-proof.mjs transport/output marker convention. Read existing reload_save_fixed.py for production HTTP/auth discovery and endpoint_override.py for guest virtual fixtures. Exercise actual Shot audio_sources/audio_sinks contract queries through the tray and Voice's actual input-list CLI/query. Create uniquely named virtual output and non-monitor remapped source in the guest, record authoritative inventory using explicitly addressed instrumentation, then run consumer operations with pactl, pw-dump, pw-cli and pw-link unavailable to the consumer processes. For the already-running Shot daemon, changing only the proof process PATH is insufficient: ensure the daemon's actual executable lookup cannot find those tools (reversible guest-only rename with tracked restoration, or supported owned daemon restart with a restricted PATH). Keep the established tray reload behavior intact. Verify names, descriptions, icons, monitor filtering and nonempty expected records; Shot's empty-on-error API cannot count as success. Record executable/source or bundle identity, exact argv, exit status, missing-tool evidence and structured output. Never print tokens, cookies or full process environments. Restore every moved tool and terminate/unload only owned fixtures in finally within a reserved cleanup budget. No hardware requirement, no capture engine removal, no host mutation, no custom QMP/socket transport, no editing existing proof reports. Bluetooth doctor may be included as read-only bundled-client evidence when its new artifact is present, but absent headset control is not a claimed hardware pass.

The guest endpoint experiment's high-priority null sink is now disproven as an output-only backend: `endpoint-audio-input.json` observes default input takeover both with automatic monitor fallback and with an explicitly configured non-monitor source. The input came back when the endpoint went away, with unchanged history, but that does not authorize the temporary input change. Keep this candidate unavailable to Sound pending a corrected backend and fresh evidence. `endpoint-audio-monitor.json` does prove target-monitor PCM with dominant 440 Hz left and 660 Hz right, both active owned links and a clean writer exit; no human audibility or earbud proof. Neither experiment uses the bundled client yet. Preserve these findings instead of restarting broad discovery or treating the measurement's pass status as product acceptance.

The fresh guest also completed `endpoint-audio-main.json`: a clean writer exit, SIGKILL without another Sound launch, the same continuing player returning to A, exact policy/history equality, and future fallback C all pass, including the previously invalid graph sample. Bundle manifest SHA is `79c0c81ce4834d3af3f6888dd63af3e58df514887ca9afb93370b13ca2d1fe3b`. These passing dimensions do not cancel the confirmed input takeover. This is experiment evidence, not a bundled Sound switch pass.

All three lanes remain edit-only, no build/test/lint/format/git or runtime commands, no code comments, no em-dash and no subagents. Architect regenerates derived dependencies, reviews all edits and runs the next gate centrally.

## Required reload behavior

1. Register a uniquely identified pending delivery for the canonical plugin before its save can publish an invalidation. Registration and reconciliation use the same plugin-manager synchronization. A pending ticket may initially have no published generation; that state must already defer reconciliation for that plugin. Do not publish first and register afterwards.
2. Capture the exact generation produced by this save. Existing untracked `set_config` callers keep their signatures and invalidation semantics where practical; add a tracked save entry point or guard accessor instead of widening every caller unnecessarily. Never infer this save's generation from the current global value after the save.
3. Dispatch reload without a manager/profile/delivery lock held across IPC. A transport `Handled` means the current daemon accepted a reload, not that it already applied the new configuration. Preserve the existing queued-reload protocol and independently observe application in the guest.
4. A ticket identifies plugin, request, generation and daemon incarnation. Settle only the matching ticket, and acknowledge only if the intended daemon is still current. Use existing tracked process identity where available; do not treat a stale response as consumption by a replacement process. If a separate incarnation counter is needed, update it in the owned lifecycle paths and exercise it in tests. A PID alone is not a proof across PID reuse.
5. Acknowledged generations only increase. Out-of-order replies cannot regress the watermark or remove a newer request. Keep all outstanding requests required for this invariant, not one replaceable flag per plugin.
6. Deferred work remains reachable after reconciliation. Do not advance a global watermark past skipped work and then rely on an unrelated future save to rediscover it. Prefer retaining the unresolved watermark or an explicit deferred set. Do not create synthetic configuration generations merely to wake bookkeeping.
7. Save failure, transport failure, bounded expiry and daemon replacement retire only their own request. Unconsumed published invalidations remain eligible for normal supervisor reconciliation. Never decrement the global generation or undo another request's acknowledgement. A partly completed save is still an error and must not claim consumption.
8. Preserve the existing failed-notification restart path. A failed old request must not restart a replacement daemon or discard a newer in-flight save. No daemon running remains a valid save case, with the normal startup/reconciliation path responsible for applying it.
9. Full-profile invalidation and plugin removal/replacement supersede affected deliveries. Idle behavior adds no polling loop; use the existing reconciliation/supervision opportunity for bounded expiry, with a test clock or explicit time parameter.
10. Plugin-originated config writes, import, sync and startup drain are not automatically acknowledged as applied merely because bytes were written. Preserve their conservative existing semantics unless an actual consuming-daemon witness is available and separately tested. The reproduction and this fix concern the host settings-save route.

## Proposed implementation ownership and API

The reload implementation lane exclusively owns these paths, including tests in them:

- `apps/qol-tray/src/plugins/config/mod.rs`
- `apps/qol-tray/src/plugins/config/tests.rs`
- `apps/qol-tray/src/plugins/manager/mod.rs`
- `apps/qol-tray/src/plugins/manager/runtime.rs`
- `apps/qol-tray/src/plugins/manager/autostart.rs`
- New `apps/qol-tray/src/plugins/manager/reload_delivery/mod.rs` and `tests.rs` if the state machine merits a module
- `apps/qol-tray/src/plugins/daemon_lifecycle/mod.rs` and `spawn.rs`, only for daemon-incarnation bookkeeping if existing process identity is insufficient
- `apps/qol-tray/src/features/plugin_store/server/settings/plugin_config_handlers/mod.rs`
- `apps/qol-tray/src/features/plugin_store/server/settings/plugin_config_handlers/io.rs`
- `apps/qol-tray/src/features/plugin_store/server/settings/plugin_config_handlers/notify/mod.rs`
- `apps/qol-tray/src/features/plugin_store/server/settings/plugin_config_handlers/notify/platform/unix.rs` and `fallback.rs`, only if dispatch outcomes need richer internal typing
- `apps/qol-tray/src/features/plugin_store/server/dev_services/reload.rs`, only to preserve its existing notification contract
- `apps/qol-tray/tests/config_reconcile_contexts.rs`, only for direct fallout from intentional API changes; do not add source-string assertions in place of behavioral tests

Suggested API shapes, with local naming changes allowed when justified:

```text
ConfigSaveReceipt { generation }
PendingReloadTicket { plugin, request_id, daemon_instance }
begin_config_reload(plugin_id) -> Result<PendingReloadTicket>
record_config_reload_saved(ticket, ConfigSaveReceipt) -> Result<()>
complete_config_reload(ticket, delivery_outcome) -> CompletionDecision
tracked_config_save(plugin_id, config) -> Result<ConfigSaveReceipt>
```

The pending state belongs beside `PluginManager`, which already owns reconciliation. Configuration publication remains owned by `plugins/config`. The HTTP layer coordinates through these APIs rather than owning another global registry. Keep the existing dev notification API compatible or adapt its one owned caller. No dependency or wire-format changes are planned. Report a required path outside this list before editing it.

The architect owns this spec. A second lane owns only the temporary guest proof leaves listed below. The owned edit sets are disjoint.

## Regression evidence

Meaningful production-state tests must cover:

- Accepted reload preserves the daemon while acknowledging the exact save.
- A deterministic barrier after publication and before dispatch allows reconciliation to run but prevents a restart.
- Two saves and reversed completions keep the last configuration authoritative and acknowledgements monotonic.
- Completing one request leaves a second pending request intact.
- Failed persistence and failed/expired notification leave no permanent suppression.
- Expiry is exercised with explicit time rather than long sleeps.
- A replaced daemon rejects old completion and failure paths, including equivalent PID values with different tracked identity where the implementation uses an incarnation token.
- A pending plugin does not block unrelated plugin reconciliation.
- Deferred work is revisited without another configuration mutation.
- Full-profile invalidation and plugin removal clear or invalidate the corresponding requests safely.

Reuse existing test path overrides and process fixtures. Test the state machine and real manager decisions, not only source spelling. Add debug probes for request/generation/instance and defer/complete/failure decisions using the existing trace vocabulary. Release builds must still compile cleanly.

The architect reruns the production guest settings route over at least two supervision periods, observes `SHOT_CONFIG_RELOAD` with the final value, and asserts stable PID and process start identity. Rapid saves must converge to the final value. A deliberate unhandled/unreachable notification must retain safe fallback behavior. Guest actions stay inside the disposable VM; no host desktop or audio actions.

## Default-policy experiment

The proof lane edits only:

- `/tmp/code-review/sound-runtime-proof-20260920/default_override.py`
- `/tmp/code-review/sound-runtime-proof-20260920/endpoint_override.py`
- `/tmp/code-review/sound-runtime-proof-20260920/reload_save_fixed.py`
- `/tmp/code-review/sound-runtime-proof-20260920/run-guest-proof.mjs`, only to add an optional scenario argv argument and source/command evidence without changing guest transport

It may read the prior leaf scripts and this spec, but it runs no tests, builds, formatters, guest commands or git commands. The architect runs the leaves centrally after fan-in.

`default_override.py` must produce a JSON report of an actual backend experiment, never an invented Sound implementation. Use argv-safe subprocess calls inside the guest, bounded timeouts, clear initial-state capture and `finally` cleanup. Its stages are:

1. Create virtual A/B/C outputs and establish configured A with history `[A,C]`. Capture metadata and the complete persisted default-node state.
2. Write only the effective `default.audio.sink` metadata key to B, without changing the configured key or state file. Verify whether it takes effect and remains effective long enough to observe.
3. Put the effective key back; remove A; compare fallback C and complete affected policy/history with the control. Record a refused/overwritten override as unsupported, not success.
4. Apply the override in an independently tracked child, terminate that child, and observe whether effective A returns without another launch. Include a bounded quiet period and an unrelated device event. Distinguish an ephemeral setting command that already exited from a process still holding the key; do not pretend killing an unrelated process proves anything.
5. Return independent verdicts for honoring the override, preserving policy, and no-next-launch reversion. Passing one never implies the others. Do not edit WirePlumber host configuration, restart the session manager, or attach hardware.

`reload_save_fixed.py` extends the existing real HTTP test, discovers the live tray port, reads the token only inside the guest without printing it, measures process PID plus start time, and captures applied-value probes. Cover one save and rapid successive saves, each with at least two supervision periods. Produce JSON with explicit pass/fail conditions and preserve the original configuration in cleanup. Keep each invocation within the existing guest exec timeout; use a scenario argument or bounded leaf stages if needed. Do not reimplement guest transport.

`endpoint_override.py` implements the concrete virtual-output experiment proposed by the lifecycle scout. Existing guest CLI tools are development instrumentation, never the shipped client. Create a uniquely named client-owned stereo sink through a kept-open PipeWire client, with an elevated session priority and `object.linger=false`. Explicitly create non-lingering monitor-to-target links owned by that same tracked connection. Do not write either default metadata key during the candidate operation. Capture the actual creator PID/start identity, registry client/node/link identities, both active channel paths, effective/configured metadata, and complete persisted default state. Establish configured A and history containing C before capturing the comparison baseline. Play one continuing stereo stream. Prove selection, forwarding to B, a clean writer exit and SIGKILL teardown without another Sound/client launch. After the writer exits remove A and verify fallback C, compare affected policy/history with the control, and record elapsed time. Independently test automatic-default baseline, target disappearance and an intervening external configured choice. Record the last as a limitation if a watcher would be needed; do not fake an autonomous precedence guarantee.

Use scenario arguments if necessary so guest transport timeouts cannot truncate a legitimate quiet-period observation. Keep all workload PIDs/modules uniquely tracked, all subprocess invocations argv-safe and bounded, and cleanup in `finally`. Preserve raw observations and typed failure reasons even when a step is unsupported. Distinguish inability to instantiate the candidate, an invalid measurement and a genuine behavioral failure. Do not claim that intentional fixture setup restored the guest's original hidden history: compare the seeded pre-operation policy and rely on verified VM teardown for complete disposal. The architect runs this proof sequentially with other guest calls. No WirePlumber restart, host configuration edit or hardware attach is part of this round.

The default-history test's previous failure remains valid evidence. If no candidate can set the host default through the host's own mechanism without side effects, record the unsupported backend boundary and the remaining implementation work; do not claim this upstream policy behavior was fixed by a tray change.

## First-round review and required corrections

The architect reviewed the complete production diff, new state-machine/manager tests, and all three guest leaves. The central check report is `/tmp/code-review/sound-runtime-proof-20260920/reload-round-one-check.json`. Formatting, single-source guard, UI tests, release-script tests and `cargo build --locked -p qol-tray --all-targets` passed. Strict Clippy failed on the untracked `notify_plugin_reload` helper being unused without the dev feature; Rust behavioral tests were not reached. No guest result has been claimed for this round.

Reload corrections, same owned Rust paths:

1. Remove the failure-to-restart check/use race. Completion currently decides `Restart` under the manager lock, then drops that lock before the handler reacquires it for unconditional restart. A replacement daemon or another request can arrive in that interval. Revalidate the ticket's incarnation, outstanding deliveries and consumed generation at the actual restart boundary under the same synchronization, or leave scoped unconsumed work to reconciliation. Never hold a lock across notification IPC.
2. Registration order is not publication order. Request 1 can publish after request 2. Do not restart while any required concurrent delivery is pending merely because it has a smaller request id. A failed old request also must not restart a daemon after a newer accepted save already consumed its generation. Use the current daemon's acknowledged generation as part of the decision.
3. Fail before publishing if pending registration cannot be established. Distinguish a valid no-daemon save from enrollment failure. Retire the matching request on notification snapshot errors where possible; expiry remains a bounded fallback, not the normal error path.
4. Add deterministic concurrency coverage at the production handler/dispatch boundary, not only manually sequenced state-machine helpers. Include a barrier after publication before notification, replacement/new save between failure decision and restart, reversed registration/publication, and old failure after a newer accepted save. Use a consuming fixture or applied-value witness for the final configuration. Real failed persistence coverage must invoke the failing save, not merely name a simulated outcome `SaveFailed`.
5. Fix the Clippy feature-boundary failure without a blanket allow. Preserve the dev-only caller; the fixed-string audit finds it in `server/dev_state_handlers.rs:94`, through the existing settings re-export. That caller need not be rewritten to resolve the owned helper's cfg boundary.

Proof corrections, same three leaves plus the narrowly extended driver:

1. The supervisor's full period is five seconds (`plugins/daemon_supervisor.rs`, readiness interval times light ticks). Observe at least twelve seconds after the final save, including the failure follow-up. An observed 3.7-second restart is not the period. Require follow-up identity stability in the verdict. Run one scenario per bounded invocation.
2. Replace the rapid-save membership comparison with a correct ordered-subsequence or final-convergence assertion that permits coalescing. Exercise overlapping requests followed by a deterministic final save; require that final value to be applied by the unchanged daemon.
3. Candidate endpoint selection and teardown cannot pass without valid baseline capture, complete unchanged configured/history policy, real stereo forwarding, tracked creator identity for both links and node, and the requested teardown mode actually occurring. Reject null/unreadable state comparisons and pre-failed setup. Automatic-baseline coverage must require absence of the configured key; do not label a previously configured guest as automatic.
4. Use observed node/port ids and channel identities for the development link commands. Preserve raw evidence. A test that samples graph links does not prove audible samples at the output; label its evidence accurately.
5. Enforce one overall execution deadline inside every subprocess/wait, with cleanup and report time reserved. Checking only between stages permits one cycle to overrun the guest's sixty-second transport limit. Track partial fixture creation immediately, remove unloaded modules from the cleanup set, and do not silently restart an expired player while claiming a continuing stream.
6. The effective-key death test must separate successful measurement from a supported cleanup capability. A completed measurement is not a passing reversion result. Prior configured-default playback evidence does not prove direct effective-key override playback; test it or leave that capability explicitly unmeasured.
7. Add a single optional fourth scenario argument to the existing argv-safe driver and record leaf hash plus exact invocation in its report. Use that path instead of stale guest marker files. Keep prior three-argument compatibility and preserve old evidence. This is an extension of the existing input-to-report node, not a new guest transport.

## Literal audit

Before this spec, fixed-string workspace searches covered `Config saved`, `Config saved but live daemon refresh failed`, `PLUGIN_RELOAD`, and `stage=ack scope=single consumed_generation=`. The HTTP text and probe identity remain unchanged. Relevant probe payload changes are owned by the reload lane in `plugins/manager/mod.rs` and `runtime.rs`. The unrelated task-runner message and CLI reload interval identifiers remain untouched. Every additional removed/changed literal must be reported with fixed-string search hits; an out-of-scope hit does not authorize editing that file. The architect repeats the final audit against the actual diff.

## Central gate and acceptance

### Tray prerequisite verified; bundled client implementation round

#### Bundled-client first review corrections

The architect read the new native client, protocol fixtures, consumer diff and proof changes. Dependency resolution added only pulseaudio 0.3.1, enum-primitive-derive 0.3.0 and the new internal edges to Cargo.lock. `cargo hakari generate` reported no changes. The central `audio-round-one-check.json` passed formatting, guard, UI and release-script checks, then failed compilation because the fake server's `writer` binding is immutable while borrowed mutably (`libs/audio/src/platform/linux/tests.rs:122`). The separate consumer lint transcript `audio-round-one-consumer-clippy.log` identifies three `clippy::useless_vec` failures in Shot's mapping tests. No test or guest pass is claimed for this round.

Audio correction keeps the same owned audio paths and root manifest; derived Cargo.lock remains architect-owned. Required changes:

1. Fix the fixture compile error and make fake-server cleanup bounded even if a test fails before connection or while the server awaits a frame. Tests must inject a synthetic cookie; they currently read the real user's cookie through `connect_at`, which is unnecessary fixture coupling.
2. Use Pulse runtime semantics: `PULSE_RUNTIME_PATH` already names the Pulse directory, so its socket is `native`, while XDG runtime uses `pulse/native`. Do not silently fall back to a different server when an explicit runtime/server override fails. Reject malformed/relative unsupported server specifications explicitly. Ensure local client configuration and cookie lookup preserve explicit precedence, including XDG config location; do not silently bypass an explicitly selected cookie. Bound cookie reads and distinguish absent cookie from malformed/unreadable explicit cookie. On native PulseAudio, authentication framing requires a 256-byte cookie even when same-user credentials provide authorization; an empty blob is not valid. Keep secrets out of errors and test recordings.
3. Profile availability is a boolean in the native card-profile wire format, not the port-availability enum. Zero means unavailable and nonzero means available. Protocols before 29 omit the field; expose Unknown for that case rather than interpreting the decoder's synthetic zero as a real observation. Add known-wire-value fixtures; do not mirror the current incorrect 1/2 mapping.
4. Socket SO_RCVTIMEO/SO_SNDTIMEO alone are per-I/O stalls, not operation deadlines. A partial/trickled reply can keep a request alive indefinitely, and blocking AF_UNIX connect can wait on a full listen backlog. Implement a monotonic deadline covering connect, authentication and each complete request (including writes and every decoder read), with prompt bounded failure and no immortal timeout thread. Add slow-fragment and backlog/no-accept fixtures in addition to the silent-server case.
5. Make event cancellation usable by the caller while a receiver is blocked. The current owner can only be dropped after `recv` returns. Add an explicit cloneable cancellation handle or equivalent stop mechanism suitable for the Bluetooth watcher's shutdown, preserving idle blocking and bounded memory. Prove idle cancellation, saturated-queue cancellation/drop, disconnect, and malformed/partial subscription input do not strand reader threads. Keep the already frozen devices API unchanged; document the additional event signature in the lane report.

Primary protocol evidence: PulseAudio v17 `src/pulsecore/core-util.c:1667` resolves the runtime directory; `src/pulsecore/protocol-native.c:2363` validates native cookie framing and `:3069` encodes profile availability as `(p->available != PA_AVAILABLE_NO)`; `src/pulse/introspect.h:522` defines the public boolean meaning. Source URLs: `https://raw.githubusercontent.com/pulseaudio/pulseaudio/v17.0/src/pulsecore/core-util.c`, `https://raw.githubusercontent.com/pulseaudio/pulseaudio/v17.0/src/pulsecore/protocol-native.c`, `https://raw.githubusercontent.com/pulseaudio/pulseaudio/v17.0/src/pulse/introspect.h`. The pinned Rust crate's `CardInfo` decoder substitutes zero below protocol 29, so negotiation must inform translation. Fixed-string searches assign the runtime resolver and availability mapping hits to this audio lane.

Consumer correction owns only `plugins/shot/src/platform/linux/system.rs` this round. Replace the three needless test Vec allocations with arrays, preserving every assertion and production behavior. No allow attributes or blanket auto-fixes.

Proof correction owns only `/tmp/code-review/sound-runtime-proof-20260920/endpoint_override.py`. Before input-policy comparison, wait within the existing budget for the fixture's configured source and its persisted source history to settle; otherwise a delayed fixture save is attributed to the endpoint. Record complete source/configured/effective baseline and settled state after the writer exits. The `input` scenario currently starts no player but demands active forwarding links: give it a real continuing player when asserting active playback, or explicitly measure the idle graph without claiming active playback. Preserve the two distinct input cases. Ensure cleanup escalates a capture that ignores terminate to kill and reap within the total reserve, rather than leaving it running after a TimeoutExpired. Keep graph-only, captured monitor samples, and human audibility separate; update the stale `no capture ... taken` evidence text for the monitor scenario. The passing graph-sampling changes and default_override.py need no edits this correction.

The fourth correction passes the direct repository commands: build, strict Clippy, 1469 tests (two skipped), doctests, and strict Clippy with the dev feature. Format, single-source guard, UI and release-script checks passed in `reload-round-four-check.json`. Its build wrapper reported surviving descendants even though the identical direct Cargo build exited zero; retain this tooling limitation without redesigning the runner. Exact commands, logs and source hashes are in `/tmp/code-review/sound-runtime-proof-20260920/reload-round-four-direct-verification.json`.

Artifact-backed guest `sound-fixed-proof-20260920`, bundle SHA `7ff5a9d8e8fea9a92b0efdc281acc66a2f3a32c067d9ba28b16c77c3282ce887`, passed real HTTP single save, overlapping saves with final convergence, and killed-daemon recovery plus a subsequent applied reload. Each relevant observation exceeded twelve seconds and checked PID plus process start identity and actual `SHOT_CONFIG_RELOAD` values. Reports: `reload-fixed-single.json`, `reload-fixed-rapid.json`, `reload-fixed-failure.json` under the evidence directory. The first script could not restore an initially absent field through the merge-save API; it reported that accurately. Full guest disposal supplies cleanup, not that failed field-level restore. Aggregate teardown is verified-cleanup and `qol env runs` is empty.

On PipeWire 1.0.5 / WirePlumber 0.4.17, the client-owned endpoint experiment passed the configured-default clean-exit and SIGKILL teardown cases, with both owned stereo links active and the same player returned to A, and byte-identical persisted default history. The automatic baseline and target-removal scenarios also passed their measured conditions. External configured choice does not displace the elevated endpoint until that process exits, proving a watcher is required. Direct effective-key writes stayed after the writer died, which the rule treats as expected: the host default is the user's change and is never reverted. These are backend experiments, not bundled-client or hardware proof. No audible samples were captured yet. `endpoint-fixed-main.json` fallback measurement is invalid because `pw-dump` emitted multiple JSON values during graph change; its confirmed C fallback and state digest do not excuse the missing metadata observation. The automatic baseline also observed `default.audio.source` change to the endpoint, which needs a separate input-side-effect check before enabling that backend.

Next round implements the bundled Pulse control client and two real consumers while correcting that measurement. PipeWire graph implementation, the Bluetooth adoption gate and Sound remain subsequent required work. This is an implementation dependency slice, not a reduced product delivery, and nothing ships independently.

Audio lane owns exactly `Cargo.toml`, new `libs/audio/Cargo.toml`, new `libs/audio/src/**`, new `libs/audio/tests/**`, and new `libs/audio/examples/**`. Add the workspace `qol-audio` path dependency. Implement Linux native Pulse protocol control with pinned `pulseaudio = "=0.3.1"`, confined to target-specific dependencies. No host pactl/CLI calls, libpulse linkage, or PipeWire client-library dependency. Use the public protocol layer for missing high-level operations instead of spawning tools. Observe the existing shared library and platform facade conventions. Other platforms return typed Unsupported from the shared boundary; existing consumers only invoke it from Linux adapters. Do not add a binary service or a host-to-plugin channel.

Freeze the concurrent consumer interface as follows (public types derive Debug, Clone, PartialEq and Eq where meaningful):

```rust
pub enum AudioError {
    Unsupported,
    ServerUnavailable(String),
    Authentication(String),
    Protocol(String),
    Timeout,
    Operation(String),
}
pub mod devices {
    pub enum Direction { Input, Output }
    pub enum State { Running, Idle, Suspended, Unknown }
    pub struct Device {
        pub index: u32,
        pub name: String,
        pub description: String,
        pub properties: std::collections::BTreeMap<String, String>,
        pub active_port: Option<String>,
        pub monitor_of_sink: Option<u32>,
        pub state: State,
    }
    pub fn list(direction: Direction) -> Result<Vec<Device>, super::AudioError>;
    pub fn default_name(direction: Direction) -> Result<Option<String>, super::AudioError>;
}
```

Empty descriptions fall back to the device name. Preserve properties and active port for the existing icon classifier. Translate the Pulse invalid-index sentinel into None for monitor identity; ordinary microphone sources must not become monitors. `AudioError` implements Display and Error with useful operation context but no credentials. Connection, authentication and requests have bounded deadlines; missing server or broken protocol returns an error, never fake empty success. Honor the existing local Pulse server/socket and authentication configuration safely. Reject unsupported remote endpoints explicitly rather than silently selecting another server. Environment default-source override remains consumer policy in Voice.

The same audio lane also implements typed low-level cards/profiles, sink-input/source-output inventories, sink suspend/resume, configured default sink setter, and event subscription needed by the already inventoried Bluetooth paths. Keep these beneath a `control`/`events` facade with public typed results, leaving Sound choice and repair policy out of this layer. A configured setter is a low-level host operation; Sound runs it at the user's direction. Subscription must block when idle, cancel promptly on Drop, bound queue growth, and report connection loss; no sleep-poll loop or immortal thread. Read-only diagnostic examples can expose inventory/server facts for the later guest, with no automatic mutation and no dependency on the tray. Add behavioral protocol fixtures for authentication/server errors, multi-item replies/end markers, properties/monitor mapping, timeout/disconnect and event cancellation as appropriate; do not test against host audio. Report exact public signatures for the later Bluetooth consumer lane.

The audio lane may not edit Cargo.lock or workspace-hack while the consumer manifests are being edited. The architect resolves and regenerates derived dependency artifacts centrally after fan-in using Cargo/Hakari, before the locked gate. No lane builds or runs tests.

Consumer lane owns exactly:

- `plugins/shot/Cargo.toml`
- `plugins/shot/src/platform/linux/system.rs`
- `plugins/voice/Cargo.toml`
- `plugins/voice/src/listen/platform/linux/mod.rs`
- `plugins/voice/src/listen/platform/linux/diagnostics.rs`

Use target-specific `qol-audio.workspace = true` and the frozen API. Remove those paths' pactl execution and parser-only plumbing; preserve each plugin's public query shapes, monitor filtering, default marking, descriptions and existing icon classification through `qol_config::contract::audio_device_picture`. Shot's existing Vec-returning adapter keeps its empty-on-failure contract but logs useful failure context; Voice retains explicit errors and honors `PULSE_SOURCE` before native default resolution. Preserve capture/probe behavior and its existing parec/ffmpeg/GStreamer dependencies. Update mapping tests to real typed device records, retaining monitor and icon cases. Do not convert non-Linux adapters, introduce UI changes, migrate Bluetooth prematurely or broaden ownership. Fixed-string `pactl`, `parse_pactl_devices`, and `parse_input_devices` searches find all Shot/Voice hits inside this set. Bluetooth hits are assigned to its later migration, not this lane.

Proof lane owns exactly `/tmp/code-review/sound-runtime-proof-20260920/endpoint_override.py` and `/tmp/code-review/sound-runtime-proof-20260920/default_override.py`. Correct graph sampling to handle observed `pw-dump` update output or retry a bounded fresh snapshot until a coherent array is obtained. Never substitute null metadata on a parse error and then classify it as behavior failure; retain raw failed attempts and mark exhausted sampling invalid. Keep policy equality fail-closed. Extend the endpoint proof with bounded capture of the chosen target's monitor and channel-specific measurements of the generated 440 Hz left / 660 Hz right stereo signal, preserving one continuing player, active owned links, cancellation and the 54-second total budget. A new named scenario is permitted; existing argv transport is unchanged. Add an input-side-effect scenario that records effective/configured source and input history with an actual non-monitor source and with monitor-only automatic fallback; classify changes honestly. Do not claim this proves human audibility or earbud behavior. Clean up every owned capture process/module. No changes to guest orchestration, host audio, WirePlumber configuration, existing evidence reports or the now-passing tray code.

### Second-round evidence and fixture correction

Fourth correction evidence: `/tmp/code-review/sound-runtime-proof-20260920/reload-round-three-check.json` records a build-runner failure (`command exited while descendants remained in its owned process tree`) and the compiler's unused-assignment warning in `wait_applied_value`. The follow-up central focused nextest run compiled successfully and exercised all eight handler cases: seven passed, with only `failing_save_retires_its_request_and_reconcile_still_applies` failing its generation assertion. Transcript: `/tmp/code-review/sound-runtime-proof-20260920/reload-handler-tests-round-three.log`.

The correction still owns only the handler `mod.rs`. Remove the unused initial assignment without suppressing the warning. Correct the failed-save oracle with the actual save pipeline in mind: `io::save_plugin_config` first calls `get_config` before invoking `set_config_tracked`; placing a directory at runtime config can fail during that pre-read, before publication. Preserve coverage of that early failure with no invented invalidation expectation, and exercise an actual persistence error after generation publication through a deterministic isolated filesystem fixture. Prove pending retirement, correct authoritative retained data, and subsequent successful reload/reconciliation without indefinite suppression. Do not merely remove the failing assertion and the rest of the meaningful coverage. No production behavior changes or unowned edits are authorized for this correction.

The central gate `/tmp/code-review/sound-runtime-proof-20260920/reload-round-two-check.json` passed build, strict Clippy, formatting, guard, UI and release-script checks. Nextest reported 1463 passed, six failed and two skipped. The six new daemon-dependent handler tests all fail during fixture loading, before their behavioral assertions: `daemon.socket must be an absolute path`. The focused transcript is `/tmp/code-review/sound-runtime-proof-20260920/reload-handler-tests-round-two.log`. This is not passing race coverage.

The next reload correction owns only `apps/qol-tray/src/features/plugin_store/server/settings/plugin_config_handlers/mod.rs`. Declare a valid absolute contract socket and resolve it through the existing isolated test-root mapping consistently for daemon notification and the scripted listener. A fixed-string workspace search found `config-reload-handler.sock` only in this owned file. Preserve all eight behavioral cases and their production boundary. Review the same fixture's witness race: observing a file is not proof it contains the expected complete JSON from the intended consumption; wait for the expected value within a bound, retain useful failure evidence, and prevent stale startup writes from satisfying later application assertions. Bound listener/read/release waits and ensure fixture shutdown on assertion failure. Keep production semantics unchanged in this correction; report an independently demonstrated production defect rather than weakening tests.

The independent backend experiment did not execute. Its report `/tmp/code-review/sound-runtime-proof-20260920/endpoint-auto-round-two.json` records `run report has no guest image revision`. Generic `env up` omits the revision that verified guest control requires; the next execution uses the already-planned artifact-backed guest. Do not modify the environment tooling or its identity checks for this task. The generic guest was stopped through `qol env down`, and `qol env runs` reported none running.

The next proof correction owns only `/tmp/code-review/sound-runtime-proof-20260920/run-guest-proof.mjs`. Preserve the exact command, source digest, raw output and failure in its report, but print a concise failure summary and report path instead of rethrowing an error whose message embeds the entire base64 guest program. Keep nonzero exit status, the existing transport, scenario argv and success parsing unchanged. No additional scouting or execution belongs to either lane.

No lane runs checks. After all writers finish, the architect personally reads every changed file against this spec. Run repository-owned deterministic formatting only on owned changed paths. Run the relevant `qol check` scope or its repository-defined build, tests, strict lint, format and single-source checks with exact argv, exit status and source fingerprint in a machine report. Do not replace behavior evidence with a source-string test or a lane's completion claim.

Then build the artifact-backed guest from the worktree and run the production save regression and backend experiment sequentially through the existing guest-control surface. Preserve bundle identity, guest version, probes, verdicts and verified cleanup. Correct defects in the owning lane, then repeat only the necessary central gate for the revised source.

Acceptance of the first implementation round requires the tray regression and concurrency cases to pass, a measured default-backend feasibility result and clean guest teardown. This does not close the full Sound task. Continue through bundled client, consumer migration, Sound commands/daemon/settings, the Bluetooth adoption gate, and the design's runtime gates. Unsupported capabilities and unavailable hardware evidence remain explicit outstanding requirements, never inferred successes. Final delivery is a scoped local commit after the applicable gates pass. Do not push.

## Guest findings, 2026-09-20

Two questions were settled in a Linux Mint guest running PipeWire 1.0.5 and
WirePlumber 0.4.17.

**A write to `default.audio.sink` at subject 0 is honoured and persists.**
Reports: `metadata-lifetime-round-one.json` and `-round-two.json` under
`/tmp/code-review/sound-runtime-proof-20260920/`.
Writing at subject 0 moves the effective default and outlives the client that
wrote it.
That persistence is expected under the product rule: the host default is the
user's change, qol records no lifetime for it and never reverts it.
Writing the same key at a non-zero subject, whether a node id or the writing
client's own global id, is stored but never acted on, so the default does not
move.
A property written at a client's own global id disappears when that client is
killed, which is `module-metadata` clearing every property whose subject is a
removed global.
Sound does not use that owner-death behavior, because it records no lifetime.
The switch resolves the requested output, sets the host default, reads the
effective default back and reports success or the real error.

**The pure-Rust graph client was never being shown anything.**
It sent `hello` and then asked for the registry, and the server answered with an
empty registry, so every lookup failed as though the machine had no default
metadata object.
A client that has not sent its properties is shown no globals at all.
The client now sends `client.update_properties` during the handshake; the
registry went from 0 globals to 92 against a live server, and the effective
default it reports matches `pactl get-default-sink`.
That client is not delivered in the tree, and the graph module remains open.
