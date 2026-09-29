# PointZ trust moves into core

Status: selected implementation contract. The [design](2026-09-27-linked-devices-design.md)
owns the migration requirements; this document selects the mechanics.

## What changes for the user

Phones that are already paired keep working without pairing again. The PointZ
settings page still shows "Pair a device" and the pairing code, but the code,
the paired phones and their keys now belong to the same core authority as linked
computers. Core settings list the paired phones and can remove one. Removing a
phone rejects its next command immediately.

## Ownership

| Fact | Owner after cutover |
| --- | --- |
| PointZ server seed and the `server_id` derived from it | Core authority snapshot |
| Paired phones: device id, device key, name, paired time | Core authority snapshot |
| Pairing window, code and attempts | Core PointZ adapter, in memory |
| UDP discovery (45454) and command (45455) sockets | Core PointZ adapter in the tray host |
| Command envelope check, clock skew and replay window | Core PointZ adapter |
| Command decoding, input bounds and input execution | PointZ plugin |

The PointZ wire protocol stays byte-identical: `DISCOVER`, `PairHello`,
`PairOffer`, `PairConfirm`, `PairResult`, `PairError` and the version 2 command
envelope. It is a scoped compatibility transport. A PointZ device holds no
operation grant and cannot reach any other plugin. Its only capability is
delivering PointZ input commands to the local PointZ plugin.

## Storage

Snapshot version 5 adds an optional `pointz` section. Version 4 snapshots open
unchanged and are written back as version 5 on their next mutation. An older
core refuses a version 5 snapshot, so a downgrade cannot ignore phone trust.

The section holds the 32-byte seed, at most 64 devices and the import outcome.
Its presence is the migration marker: legacy PointZ files are read only while
the section is absent, so a removed phone can never come back from a leftover
file. Removing a phone deletes its record; pairing it again is a new, locally
approved pairing. Device names are
sanitized on entry: control characters are dropped, the name is trimmed and
bounded to 256 bytes, and an empty name becomes "Phone". Device ids are unique. Keys and the seed are zeroized and never
appear in projections, errors or traces.

## Import

Import runs once, inside one authority mutation, when all of these hold: an
authority is active, the installed PointZ declares core trust, no PointZ daemon
from a legacy manifest is running, and the `pointz` section is absent.

1. Read `pairing-secret` and `devices.json` from the PointZ data directory
   without modifying them. A missing or unreadable seed produces a new seed and
   a visible "phones must pair again" outcome. An unreadable device file imports
   no devices and reports the same outcome.
2. Keep the 64 most recently paired devices if there are more, and report the
   number dropped.
3. Commit the section with its import outcome. Nothing is acknowledged before
   the commit succeeds; a failed commit faults the authority exactly as other
   mutations do.
4. After the commit, rename both legacy files with a `.migrated` suffix. A failed
   rename is traced and changes nothing: the marker already prevents reimport,
   so a leftover file can never restore a removed phone.

A host with no legacy files creates the section with a fresh seed the first time
the user begins PointZ pairing.

If no authority is active, legacy files exist and persistent storage is
supported, the host creates a persistent authority named after this computer as
a controlled migration. Beginning PointZ pairing does the same, because the user
asked for it. Neither path runs on a standby development host.

## Legacy writer exclusion

The installed PointZ declares `peer_trust = "pointz-v1"` under `[capabilities]`.
Plugin API validation accepts only that value.

- Legacy PointZ and no `pointz` section: PointZ keeps its own pairing until it
  is updated. Core does not bind its ports. Linked devices status says so.
- Legacy PointZ after cutover: the host refuses to start its daemon and reports
  that PointZ must be updated. It never falls back to the legacy registry.
- Compatible PointZ: the plugin binds no discovery or command socket and writes
  no trust file. Its manifest drops `[[daemon.extra_ports]]`.

## Adapter lifecycle

The core adapter runs while the authority is active, the section exists and a
compatible PointZ plugin is installed and enabled. It binds both UDP ports on
`0.0.0.0` with broadcast enabled. A busy port is a visible failure, not a retry
loop. Stop, authority fault, plugin removal and host shutdown close both sockets
and cancel the pairing window before the writer is released.

Verified commands go to the PointZ daemon socket as one `input` request each,
sent in arrival order from a single forwarding task with a bounded queue of 256.
A full queue or an unreachable daemon drops the command, as UDP loss would, and
counts the drop for status. The payload must be a JSON object of at most 4096
bytes; core never interprets it.

## Pairing

At most one pairing window exists. It lasts 60 seconds, allows three wrong codes
and 16 pending phones, exactly as before. A successful confirmation commits the
device before `PairResult` is sent. If the commit fails, the phone receives
`PairError` with reason `unavailable` and holds no key.

## Local controls

Canonical admin requests gain a `pointz` group: status, devices page, begin
pairing, cancel pairing and remove device. Removal carries the expected
authority stamp. Status reports the import outcome, `server_id`, device count,
pairing window (code only while open), socket state, drop counts and whether the
installed PointZ is compatible. `qol peers pointz` exposes the same requests.
Linked devices settings list paired phones and remove them; PointZ settings
keep the pairing button and code, served by the PointZ daemon asking core.

## Evidence

Tests use temporary stores, generated keys and loopback ports only:

- import preserves `server_id`, keys and names; corrupt files, oversize lists and
  a missing seed produce the reported outcomes;
- a second start never reimports, including after a removal and with the legacy
  files restored by hand;
- a removed phone is rejected on its next command through the real adapter;
- a simulated phone pairs through the real UDP adapter with the existing client
  handshake, then its command reaches a fixture PointZ daemon;
- a failed pairing commit sends no key; replays and stale timestamps are refused;
- a legacy PointZ manifest after cutover cannot start;
- version 4 snapshots open and upgrade.

Phone verification with the Flutter client happens on real hardware.

`TRAY_PEERS` traces import outcome, adapter start and stop, pairing open, close
and result, device removal and drop counts. Codes, keys, seeds and payloads never
appear in traces.
