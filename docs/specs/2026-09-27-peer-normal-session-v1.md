# Normal peer sessions

Status: selected intermediate implementation. This establishes authenticated
session mechanics; the host network supervisor, request admission, PointZ
migration, and Bluetooth remain required for delivery.

## Authority and admission

Normal sessions use the existing PeerAuthority and normal TLS constructors.
The authority resolves an outgoing PeerId to its currently trusted pin; callers
do not supply a replacement pin. Expose a general authority-change subscription
from the existing watch sender. No second registry, identity, writer, or trust
cache is created.

An established session owns its verified PeerConnection and a handle to that
same authority. Reject enrollment ALPN, absent or revoked peers, local-peer
connections, and a faulted authority before exchanging normal messages. Check
current authority state after the handshake, before accepting frames, and while
the session is idle. Revocation or an authority fault drops the connection.

The host will own each session future and supply cancellation. This library
round opens no production listener, spawns no detached task, and dispatches no
plugin operation. The transport cannot imply an operation grant. Request
admission and durable replay state precede any future invocation variant.

## Session generation

Each side generates a fresh 128-bit nonce for every normal connection, encoded
as canonical unpadded base64url. A strict version-1 hello carries the sender and
recipient peer IDs and this nonce. IDs must match the actual TLS identities and
the local authority. Both directions exchange hello under a five-second total
deadline; bounded duplex transports must not deadlock while both sides write.

The pair of local and remote nonces identifies this connection. Subsequent
frames name the sender and recipient nonces in their correct direction. A
different or old generation closes the session. The host activation ID guards
local commands; the connection nonce pair guards network frames. These are
different lifetimes, with one owner for each.

Only a validated received hello permits an authenticated-session projection.
A write, discovery hint, or previous session's event cannot establish it. The
later host supervisor must compare the full connection generation before
applying an event and invalidate the projection when its session ends.

## Framing and liveness

Use the existing bounded JSON codec and 65536-byte normal frame limit. All
version, tag, field, duplicate-key, UTF-8, depth, and value bounds still apply.
Freeze fixtures for hello and a generation-bound heartbeat; no arbitrary
payload or invocation escape hatch is exposed in this round.

Keep the enrollment and handshake frame API's five-second deadline. Add a
normal-session read mode that waits for the first prefix byte, then enforces
the same five-second deadline for the rest of that frame. Both modes share
one payload decoder and validator. Normal idle waiting is separately bounded.

An active normal session sends a heartbeat every 15 seconds and closes after
45 seconds without a valid inbound heartbeat. This bounds unreachable-state
staleness. Timer work exists only while a session is active; there is no empty
service poll. Missed ticks do not burst. Bound inbound control-frame rate to
64 per 60-second monotonic window and close on excess.

Reader and writer operations retain partial-frame state while waiting for
unrelated events. Cancelling read_json in a recurring select loop and starting
over on the same stream would corrupt framing; only cancellation that discards
the entire connection is allowed. Shutdown, revocation, malformed input,
timeout, excess rate, or either I/O direction failing ends the whole session.
All errors are fixed categories without payloads, names, secrets, or paths.

## Evidence

Use actual mutual TLS and generated fixtures over in-memory transports. Test
trusted admission, enrollment rejection, wrong IDs and generations, revocation
before and after opening, authority faults, local cancellation, quiet periods,
idle expiry, fragmented and trickled frames, duplicate/extra fields, oversized
frames, rate bounds, and reconnect generation changes. Prove an authority
notification or heartbeat cannot discard a partially received frame. Test both
ends closing and release of their owned transport resources. No host peer
store, network listener, Bluetooth device, or PointZ record is a fixture.
