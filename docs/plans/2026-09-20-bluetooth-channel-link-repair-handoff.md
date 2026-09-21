# Bluetooth playback channel link repair handoff

## Objective and user expectations

Make qol detect and repair a Bluetooth playback stream whose per-channel PipeWire link is stuck in the `paused` link state, where one earbud is silent while every mixer-level check (channel map, volumes, mute, profile, codec) looks healthy.

The user asked for this explicitly: "I would like qol to be able to fix this issue."
The deliverable is tray-owned detection plus a bounded, visible repair path, not a documented workaround the user has to run by hand.
Do not close this as done because a manual `pw-link` sequence works once.

The user spent a long back-and-forth on 2026-09-20 while the earbuds were fine and the desktop mixer was fine.
Anything that reports "no problem found" while a channel link is paused is a false negative and must be treated as a defect in the check itself.

## Symptom as the user sees it

The right earbud of a soundcore P40i pair plays from the phone and from a PC test tone, but a Firefox video on the PC comes out of the left earbud only.
Both earbuds are connected to the PC and the phone at the same time (multipoint).
The user hears the left channel in both the sink and the app is otherwise normal: no mute, no balance change, no codec switch, no reconnect needed before the fault.

## Machine and stack (live-verified 2026-09-20, PC Alpha)

- Linux Mint 22.3 Zena (Ubuntu 24.04 base), kernel 6.8.0-139-generic, Cinnamon on X11.
- PipeWire 1.0.5, WirePlumber 0.4.17, libpulse 16.1.0, `pactl` reports `PulseAudio (on PipeWire 1.0.5)`.
- Bluetooth audio device `soundcore P40i`, A2DP sink, codec SBC, connected on `hci0` (add-in Intel AX210 controller; the onboard controller was also powered).
- Device address and BlueZ object paths are redacted here; read them live with `bluetoothctl devices Connected` and `busctl --system tree org.bluez`.
- Machine-specific history for this host lives in the qol-skills repo, plugin `personal-computers`, reference `pc-alpha.md`.

## Root cause

The fault is a PipeWire link-object state, not an audio-level problem.

The failing graph, captured while the user reported left-only audio:

```text
$ pw-dump | jq -s 'add | .[] | select(.type=="PipeWire:Interface:Link")
    | "\(.id) out=\(.info["output-port-id"]) in=\(.info["input-port-id"]) state=\(.info.state)"'
"88 out=79 in=71 state=active"
"84 out=78 in=70 state=paused"

$ wpctl status   # Streams section
85. Firefox
    78. output_FR  > soundcore P40i:playback_FR  [paused]
    79. output_FL  > soundcore P40i:playback_FL  [active]
```

A `paused` PipeWire link carries no buffers, so the right channel never reached the sink while the left channel kept flowing.
PipeWire link states are `error`, `unlinked`, `init`, `negotiating`, `allocated`, `paused`, `active`; only `active` moves audio.
The left link of the same client stream was `active`, which is what makes this so confusing to diagnose from the mixer side.

The same object inspected directly:

```text
$ pw-cli info 84
  output-node-id: 85
  output-port-id: 78
  input-node-id: 69
  input-port-id: 70
  state: "paused"
  format: F32P (dsp)
  properties: object.linger = "true"
```

## Why no existing check catches it

- `pactl` cannot see it. The sink input reports `Channel Map: front-left,front-right`, equal per-channel volumes (both `52700 / 80%` at the stream and both `44895 / 69%` at the sink), and `Corked: no`. All of that is identical to a healthy stream.
- The sink monitor is not a valid probe. The sink adapter's channelmix `upmix` feature duplicates the left channel into the empty right channel, so `pw-record --target=<sink>` shows two identical channels with equal RMS while the right link is dead. I initially read that capture as evidence of a healthy right channel and it was wrong.
- `audio_claim` (`plugins/bluetooth/src/audio_claim/`) only detects that playback started on a Bluetooth sink and nudges it with `pactl suspend-sink 1/0` (`backends/pulse_streams.rs::suspend_resume`). It never reads PipeWire link state.
- The daemon `audio_watch_tick` (`plugins/bluetooth/src/platform/linux.rs:2692`) repairs a degraded A2DP profile only. A paused channel link leaves the profile at `a2dp-sink`, so the watch stays idle.
- Doctor checks in `plugins/bluetooth/src/cli.rs:311` cover BlueZ availability, adapter power, config readability, helper binaries, managed devices, and host takeover. None of them inspect the audio graph.

## Evidence that isolates the fault

Verified fine:

- Mixer and adapter volumes balanced per channel, sink not muted, stream not corked.
- The BlueZ sink node is stereo: `Format` is `S16LE`, 48 kHz, 2 channels, positions `FL,FR`, and `EnumFormat` offers exactly that.
- The A2DP transport is SBC: `busctl --system get-property org.bluez <transport> org.bluez.MediaTransport1 Codec` returns `y 0`; `Configuration` returns `ay 4 17 21 2 43`; `State` is `active`; `Volume` is `q 87`. Decode the configuration bytes against BlueZ `a2dp-codecs.h` before drawing conclusions from them.
- A `paplay` test tone (a fresh client stream) negotiated `active` links on both channels and the user heard it in both earbuds. Test file and sequence: `paplay /tmp/chtest.wav` with a 3-beep pattern (660 Hz both, 880 Hz right only, 440 Hz left only).
- Conclusion: the earbuds, the Bluetooth RF link, the sink node, and the mixer are not the cause.

Verified broken:

- The pre-existing Firefox stream had exactly one `active` and one `paused` channel link, persisted across repeated snapshots and across a Firefox video change (two Firefox streams existed later; the newer one had both links `active`).
- Repair attempts:
  1. `pw-cli destroy <FR link>` then `pw-link Firefox:output_FR <bt sink>:playback_FR`: the link reported `active` briefly, then fell back to `paused`.
  2. `pactl move-sink-input <id> <other sink>` then back: the FR link stayed `paused` and the FL link disappeared. WirePlumber never restored a full active pair.
- Conclusion: relinking an already-wedged client stream is not a durable repair. The client stream must be recreated (application side), or the sink node rebuilt (device side).

Not tested, and the implementer should test them:

- `pactl suspend-sink <sink> 1` / `0` against a paused link.
- A sink profile renegotiation (`a2dp-sink` -> `headset-head-unit` -> `a2dp-sink`).
- A full Bluetooth reconnect that rebuilds the sink node.
- Whether the paused link is the cause or a symptom of the client having stopped writing one channel.

## Trigger hypothesis (unconfirmed)

The most likely trigger is a WirePlumber relink race when the sink node is created or recreated while a client stream is already playing.
Supporting observations from the failure window, all from `journalctl`:

- `bluetoothd`: `a2dp-source profile connect failed for <device>: Device or resource busy`, then later `<...>/sep1/fd0: fd(64) ready`.
- `wireplumber`: `<WpSiAudioAdapter:...> failed to activate item: Object activation aborted: proxy destroyed`.
- The client stream's node `ERR` counter was growing (63, then 71) across snapshots, so xruns were present on the stream that owned the paused link.

Reproduce before believing this: bring up the headset while a long playback stream exists, force the sink to be recreated (profile toggle or reconnect), and watch link states.
A guest VM with USB Bluetooth passthrough is the closest disposable repro; see `qol-project:qol-dev-environments`.

## Source map for the fix

| Owner | Responsibility | Files |
| --- | --- | --- |
| Playback reclaim watch | `pactl subscribe` plus MPRIS, reclaims a Bluetooth sink when playback starts | `plugins/bluetooth/src/audio_claim/platform/linux/mod.rs`, `.../backends/pulse_streams.rs` |
| Repair decision, pure | Cooldown and attempt cap state machine | `plugins/bluetooth/src/bluetooth/audio_watch.rs` (`AudioWatchState`, `RepairDecision`) |
| Daemon profile watch | Interval-driven degraded-profile repair using `AudioWatchState` | `plugins/bluetooth/src/platform/linux.rs` (`audio_watch_tick` at 2692, loop at 1762 to 2045) |
| pactl parsing | `list short sinks` and friends | `plugins/bluetooth/src/platform/pactl.rs` |
| Headless surface | `reclaim` command, doctor checks | `plugins/bluetooth/src/cli.rs` (`reclaim_command` around 228, `doctor_checks` at 311) |
| Probes in use | `BLUETOOTH_AUDIO_CLAIM`, `BLUETOOTH_PROFILE_REPAIR`, `BLUETOOTH_DEFAULT_OUTPUT` | same files |

The fix belongs in the Bluetooth plugin, which already owns Bluetooth audio readiness. Do not start a second audio plugin.

## Proposed implementation

1. Detection, split pure and IO:
   - Add a Linux module that runs `pw-dump`, parses Link objects, resolves port ids to node ids and names, and reports links whose input node is a Bluetooth sink (`bluez_output.*`) and whose state is `paused` while a sibling link of the same client stream is `active`.
   - Keep the predicate target-free and unit-test it from captured `pw-dump` JSON fixtures.
   - Also flag the fully-paused case only as informational, because a stream that never started is a different condition from a stream that is playing left-only.
2. Repair ladder, bounded by the existing `AudioWatchState` cooldown and attempt cap:
   - Step 1: destroy the paused link and recreate it between explicit node and port ids.
   - Step 2: `suspend_resume` the sink (already implemented and probe-instrumented).
   - Step 3: reconnect the device through the existing reconnect path so the sink node is rebuilt.
   - Stop after the cap and leave the failure visible in doctor output and a probe row; never loop silently.
3. Surface:
   - Prefer the daemon tick to the reclaim watch, because it already runs per-device with interval pacing and repair state.
   - Add a read-only doctor check, for example `audio_links`, that reports paused channel links for connected audio devices without mutating anything.
   - Optional: a headless `repair-links` command mirroring `reclaim` for manual recovery.
4. Traps:
   - Multiple client nodes can share a name. Two Firefox streams were both named `Firefox`, and `pw-link Firefox:output_FR ...` linked every match. Operate on node and port ids, never on names.
   - `pactl` indexes and PipeWire object ids are different namespaces. Do not mix them.
   - Links created by WirePlumber carry `object.linger = true`. Verify a replacement link survives the creating process exit, or set linger explicitly.
   - `pw-dump` output can arrive as concatenated JSON documents. Parse defensively, for example `jq -s 'add'`.
   - Never move or kill a user stream as a repair. `move-sink-input` cost the healthy channel in the observed failure.
   - The repair must be reversible and must not leave host state behind, per the `qol-project:qol-mission` invariants.
   - Enrich the existing probes instead of adding a parallel trace channel, per `qol-trace-discipline`.

## Verification

- Unit tests: detection from fixtures (healthy pair, one paused, both paused, no BT sink, name collision), and repair decision (cooldown, attempt cap, exhausted).
- Live repro: guest VM plus real Bluetooth audio hardware, or host verification with the user's earbuds when a guest cannot expose Bluetooth. Assert detection, then repair, with before and after `pw-dump` evidence.
- Acceptance: the affected app's audio reaches both earbuds without restarting the app; both links report `active`; a deliberately unrepaired case is reported in doctor and probe output rather than swallowed.
- Record whether step 1 alone or step 2 alone is sufficient; a shorter ladder is better.

## Open questions

- Why the right link and not the left?
- Is the `paused` state the cause or a symptom of the client no longer writing that channel? The node's growing `ERR` counter hints at the latter.
- Does `suspend_resume`, already in the plugin, actually clear a paused link? Untested in this incident.
- Does the same failure occur for non-Bluetooth sinks, and for non-Firefox clients?
- What does the SBC configuration `ay 4 17 21 2 43` select for channel mode, and does the earbud firmware treat that mode differently from the phone's codec choice?

## Reproduction commands used in this incident

```bash
# link states
pw-dump | jq -s 'add | .[] | select(.type=="PipeWire:Interface:Link")
  | "\(.id) out=\(.info["output-port-id"]) in=\(.info["input-port-id"]) state=\(.info.state)"'

# per-stream port view
wpctl status

# link detail, including linger and format
pw-cli info <link-id>

# transport codec and configuration
busctl --system get-property org.bluez <transport-path> org.bluez.MediaTransport1 Codec
busctl --system get-property org.bluez <transport-path> org.bluez.MediaTransport1 Configuration

# mixer-side checks that show a healthy stream
pactl list sink-inputs
pactl list sinks
```

## References

- `qol-plugin-bluetooth` skill: plugin invariants, doctor read-only rule, source ownership.
- `qol-project:qol-dev-environments` skill: disposable guest workflow for desktop runtime behavior.
- `qol-trace-discipline` skill: probe target and message conventions.
- Format precedent: `docs/plans/2026-09-05-qol-memory-semantic-retrieval-handoff.md`.
