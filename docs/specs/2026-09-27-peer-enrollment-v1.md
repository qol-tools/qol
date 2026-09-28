# Peer enrollment and local authority integration

Status: selected implementation round, not a completed or deployed feature.
The [transport](2026-09-27-peer-transport-v1.md) and
[authority](2026-09-27-peer-authority-storage.md) contracts still apply.

## One authority

Enrollment extends the existing PeerAuthority. Pending invitations belong to
its in-memory state; approved links, transaction receipts, and pending outbound
joins use its existing atomic snapshot and writer. It must not create another
identity, trust store, completion marker file, or grant writer.

The host owns one handle and installs that handle into its existing runtime
server after plugin loading. Clients use an additive typed RuntimeRequest and
a qol-runtime client API. Client code has no storage constructor or fallback
that starts an independent service. Public local administration types live in
qol-peers' lightweight contract layer. AuthorityError moves there with facade
re-exports; one error type serves both existing service and local clients.

The normal host opens an explicitly created existing store from the host data
namespace, outside profiles and installed-plugin directories. Absence leaves
the feature inactive. Corruption, missing snapshot/lock, unsafe paths, and writer
contention produce an unavailable state, never an automatic new identity.
Only a local request can create a persistent authority or start a session-only
authority. Neither linking nor profile sync may change HostResidency. A shadow
dev generation does not acquire the predecessor's writer; acquisition follows
the existing promotion event. Shutdown closes the active service handle and
denies later requests. No polling retries are added to startup.

This round adds status, start-session, create/open-persistent, stop, rename,
exact-grant updates, and revocation through the existing local runtime socket.
The fixed authority root is host-supplied, never a request parameter. The socket's
existing same-user credential check is the local boundary; it does not identify
plugins and is not represented as remote authorization. Local grant editing
checks current canonical peer exposure through the host's installed catalog.
Ordinary network operation admission remains a separate later integration.

Keep the existing 64 KiB local runtime message limit. Status carries summary
metadata; bounded pages carry peers, grants, and tombstones. Page requests bind
the local authority identity and revision so a changing store or session cannot
silently mix snapshots. Do not send an unbounded full AuthorityProjection over
the socket. Oversized mutation requests fail before dispatch. A local client
timeout after sending a mutation has an unknown outcome and never causes an
automatic retry.

Local mutations of an active authority bind its peer identity, activation ID,
and expected store revision. The host generates one random 128-bit activation
ID before opening or creating each active authority; clones and projections
share it. It is not persisted, a credential, or a second peer identity. A stopped
and reopened authority receives a new activation ID even when its key and store
revision are unchanged. Page cursors carry the same activation binding. Check
these bindings under the host lock before calling the authority, so delayed
commands and page requests cannot apply to another activation.

Serde declarations alone do not establish strictness: fieldless internally
tagged variants must reject extra fields as well. Preserve canonical wire
fixtures and exercise every local request, reply lifecycle, and error shape.

## Invitation

An invitation contains a random 128-bit identifier, random 256-bit secret,
inviter SPKI pin, version 1, at most eight literal socket-address hints, and the
inviter's actual storage lifetime. Encode the small JSON document with canonical
unpadded base64url and prefix it with `qol-link:`. Bound the entire exported
document to 4096 bytes. Address hints are routing data; the TLS pin establishes
the intended identity. Reject invalid/zero ports, invalid pins, malformed or
duplicate fields, noncanonical encoded values, unsupported versions, and bounds
violations before attempting a connection.

The token has an explicit export boundary with redacted Debug and owned zeroizing
buffers. No normal projection, trace, discovery record, argv, or automatic
clipboard path contains it. Use the selected cryptographic provider's operating
system randomness and a maintained constant-time comparison primitive.

An inviter retains at most eight pending invitations, each expiring within
120 seconds on a monotonic clock. Reservation does not extend that expiry.
Cancellation or expiry immediately prevents subsequent approval. Session-only
and persistent authorities share this behavior; unused invitation secrets never
enter the persistent snapshot.

## Enrollment transaction

1. The joiner locally selects its authority lifetime, parses the invitation,
   generates a transaction identifier, and records the pending outbound join
   with the inviter pin in that authority before asking the inviter to commit.
   The outbound durable record contains no invitation secret.
2. Enrollment TLS proves both keys. The joiner pins the inviter before sending
   the secret. The inviter obtains the joiner's pin exclusively from the verified
   enrollment connection. A claimed peer ID or name never substitutes for it.
3. A valid redemption reserves the invitation to that authenticated pin and
   transaction. A different key/transaction cannot reuse it. Repeating the same
   request preserves the originally presented name and state.
4. A local approval presents the authenticated peer and each host's selected
   storage lifetime. Approval uses an expected store revision. It atomically
   inserts the peer with empty grants and a consumed transaction receipt, then
   acknowledges the durable result. Local grant editing remains a separate
   explicit operation; enrollment grants no plugin operation implicitly.
5. The joiner validates the response against its authenticated inviter and
   pending transaction, then commits the inviter link and outcome through its
   own authority before reporting local success.

The two stores do not form a distributed atomic transaction. A crash can leave
one side committed. Surface pending/unknown state and support receipt-based
reconciliation; never announce bilateral completion from a transport write.

After a lost reply or restart, recovery requires the original authenticated
joiner key, inviter pin, and transaction identifier. A committed receipt can
recover its result without the expired secret. Unknown or mismatched transactions
cannot redeem a consumed invitation. A revoked peer invalidates its receipts
and denies recovery; ordinary insertion/recovery cannot remove a tombstone.
Explicit relinking requires a dedicated locally approved transition, outside
this round. Pending outbound joins and receipts are bounded (at most 32 pending
outbound joins and 256 retained inbound completion receipts); exhaustion fails
before mutation. Do not silently evict recovery evidence or tombstones.

Malformed or duplicate enrollment records, inconsistent pins/IDs, receipts for
absent or revoked links, duplicate transactions, and capacity violations reject
store opening. Extend the snapshot version deliberately; this is still an
unreleased format, so fixtures can move together. Every uncertain write failure
faults the same authority, including pending approvals and recovery.

## Exchange boundary

Enrollment request/response structs form their own strict protocol, separate
from normal peer operations and local administration. The service exposes
exchange functions over caller-supplied AsyncRead/AsyncWrite transports and
existing TLS constructors; the host will later supply bounded TCP accept/dial
tasks. No production listener is activated in this round.

Wire exchanges use a four-byte unsigned big-endian payload length followed by
UTF-8 JSON. Enrollment frames are at most 4096 bytes; normal frames will be at
most 65536 bytes. Empty, oversized, truncated, invalid UTF-8, or trailing JSON
payloads fail the connection. Reject duplicate object keys recursively, including
escaped-equivalent keys, before typed parsing. Bound nesting to 32 levels and
the total number of JSON values to 4096. Types still deny unknown fields/tags.

The private framing module owns these mechanics once. It exposes FrameLimit
(`Enrollment`, `Normal`), FrameError, and async `read_json`/`write_json` over a
mutable transport. Each complete frame read/write has a five-second deadline;
handshake and local-approval lifetimes are controlled separately. After a framing
error, the caller discards that connection. Owned serialized/read buffers are
zeroized, and errors contain no payload data. Enrollment uses this codec rather
than maintaining a second JSON framing implementation.

Approval waiting is event driven with an expiry deadline. Do not hold an
authority mutex across await, TLS I/O, or a local user's decision. Re-check
current fault/revocation/transaction state at commit and recovery boundaries.
No enrollment message reaches an ordinary plugin handler.

Pending responses may keep the five-second enrollment frame boundary alive
while waiting for local approval. They cannot extend the invitation deadline
or turn a failed, revoked, or uncertain transaction into a successful one.
The [normal-session contract](2026-09-27-peer-normal-session-v1.md) defines the
separate idle and partial-frame deadlines for established peer connections.

## Explicit outbound recovery

Each outbound record has one authoritative state: pending, abandoned, or
committed with its receipt. Store that state in the existing snapshot; the
receipt must not also be represented by an independently mutable boolean.
Abandonment describes a local decision to stop this attempt. It makes no claim
that the inviter rolled back or that its outcome is known.

`abandon_join(expected_revision, transaction)` durably changes pending to
abandoned. Preserve the original transaction, invitation, pin, presented name,
and lifetimes. A committed link cannot be abandoned; removal uses revocation.
Repeated abandonment may return the current revision after the same revision
check, without another write. Failed or uncertain persistence retains the
existing fault behavior. No timer, rejection, or transport error abandons an
attempt automatically.

Only an explicit subsequent prepare operation may create a different attempt
for the same pin. Abandoned attempts do not occupy the one active attempt per
pin or the 32-pending capacity, but count toward the 256 retained outbound
records. All retained invitation and transaction IDs remain unique, including
abandoned records. Refuse exhaustion rather than evicting uncertain evidence.

`resume_join(expected_revision, transaction)` durably restores an abandoned
attempt to pending with its original identifiers. Refuse if the pin is revoked,
already linked, or has another active attempt, if an incompatible incoming
reservation exists, or if pending capacity is full. It does not generate a new
transaction or send a request. The caller separately redeems the original
invitation or requests recovery of that original transaction.

Redemption, recovery, and receipt application reject abandoned records. Check
again when applying a received receipt so an in-flight exchange cannot revive
an attempt after abandonment. A response lost after remote commit remains an
unknown outcome. A new attempt may conflict with that remote link; explicit
resume and recovery of the preserved original record remains available.

Snapshot validation permits abandoned records beside a later active attempt or
link for the same pin. Pending and committed records retain their distinct
link/receipt invariants, and there is at most one active outbound record per
pin. Revocation still removes all enrollment records for the revoked pin and
retains its tombstone. Bump the unreleased snapshot format deliberately, with
strict state tags and required fields. These transitions require public local
controls before the enrollment feature is complete.

## Evidence and remaining delivery

Author isolated tests for valid bilateral enrollment, initial empty grants,
wrong token/pin, key/transaction substitution, duplicate redemption, cancellation,
expiry, concurrent approval, stale revisions, commit failures, lost replies,
restart reconciliation, and revocation after commit. Exercise actual TLS and the
framing codec with generated credentials and in-memory transports. Storage tests
use temporary directories and never host trust data.

Host tests exercise the real local runtime route, unsupported platforms, competing
writers, missing/corrupt storage, explicit lifetime selection, profile independence,
shutdown, and shadow/promotion ownership. These tests establish neither full
network readiness nor PointZ migration. The full feature still requires session
management, discovery, current-catalog dispatch and replay protection, PointZ
cutover, Bluetooth coordination, UI/CLI presentation, and guest verification.
