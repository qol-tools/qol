# Earbud handoff between linked devices

Status: selected implementation contract. The [design](2026-09-27-linked-devices-design.md)
owns the requirements; the [remote operation contract](2026-09-27-peer-operations-v1.md)
owns delivery.

## What changes for the user

A paired pair of earbuds in Bluetooth settings gets a "Move here" button. It asks
the linked device that has them to let go, connects them to this computer and
checks that audio is ready. If the other computer cannot be reached or refuses,
nothing changes there and the reason is shown.

Setup happens once, on the computer that usually holds the earbuds: in Linked
computers settings, grant the other computer the Bluetooth operations
`handoff_state`, `release_for_handoff` and `resume_reconnect`.

## Ownership

| Fact | Owner |
| --- | --- |
| Peripheral identity across computers | `qol-bluetooth-control`: the normalized Bluetooth address |
| Whether QoL may reconnect a peripheral automatically | `qol-bluetooth-control` reconnect holds |
| Handoff workflow and its outcome | Bluetooth plugin on each computer |
| Controller reclaim policy | Controllers plugin, using the same holds |
| Delivery, grants and request outcomes | Core peer service; it never learns about audio |

Connection operations stay in each plugin's platform adapter. What both plugins
share is the identity and the holds, which is what keeps one writer from undoing
another.

## Reconnect holds

A hold names an address, an owner (`handoff` or `controller_reclaim`) and an
expiry. Holds live in one small file under the QoL runtime directory, written
atomically under an exclusive file lock and read under a shared one, so every
plugin process sees the same holds and a restarted daemon still honors them.
An unreadable file holds nothing and is replaced on the next write. At most 64
holds exist; expired holds are pruned on every write.

- Automatic reconnect in the Bluetooth daemon skips an address while it is held.
- An explicit connect or reconnect that the user starts on that computer,
  from settings or the command line, releases every hold for the address
  first, because the user asked for it.
- A handoff hold expires after 12 hours as a safety net. `resume_reconnect`
  releases it earlier.
- Controllers places a 30 second hold before it disconnects a stuck controller,
  so Bluetooth's automatic reconnect cannot race the controller reconnecting
  itself.

## Operations on the computer that holds the earbuds

Declared in Bluetooth's runtime contract with explicit peer exposure:

| Operation | Kind | Replay | Result |
| --- | --- | --- | --- |
| `handoff_state` | query | idempotent | `known`, `connected`, `held` for one address |
| `release_for_handoff` | action | never | places the hold, disconnects, verifies the link is down; `released` and `was_connected` |
| `resume_reconnect` | action | idempotent | releases the handoff hold; `resumed` |

`release_for_handoff` places the hold before it disconnects and reports failure
if the link is still up after five seconds. It answers within the ten second
remote limit.

## The "Move here" workflow

1. Resolve the address and read the local device. An unpaired device is refused.
2. Read linked devices from core and ask each one, at most 16, for
   `handoff_state`. A computer that refuses or cannot be reached is skipped and
   named in the outcome.
3. If one computer reports the earbuds connected, call `release_for_handoff`
   there. Anything other than a verified release stops the workflow before this
   computer touches the earbuds.
4. Connect locally with the existing verified connect, retrying three times over
   about five seconds while the earbuds free their slot.
5. If the local connect fails after a release, call `resume_reconnect` so the
   other computer can take the earbuds back, and report the failure.
6. Report the verified outcome: which computer released them and that audio is
   ready here. A remote acknowledgement alone never counts as success.

An unknown remote outcome is reported as unknown; the workflow never repeats a
release it cannot prove.

## Evidence

- Holds: expiry, owner-scoped release, user release, cross-handle visibility,
  corruption and bounds.
- Workflow with fake remote and local sides: success, nobody holds them,
  refused or unknown release, local connect failure followed by resume, and a
  peer that cannot be reached.
- Controllers places its hold before disconnecting and releases it when the
  disconnect fails.

The automatic reconnect loops on Linux and macOS check the hold before every
attempt. They drive the real Bluetooth stack, so they are checked on hardware
together with real earbuds moving between two computers.
