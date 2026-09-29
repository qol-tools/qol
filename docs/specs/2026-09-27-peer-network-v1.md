# Core peer network ownership

Status: selected intermediate round. This connects the verified normal-session
mechanics to the real host lifecycle. Enrollment listeners, operation admission,
PointZ migration, and Bluetooth remain delivery requirements.

## Authority and lifetime

The network supervisor belongs in `qol-peers` behind its service feature. It
consumes the host's existing PeerAuthority; it never opens a store or creates an
identity. Discovery endpoints and live connections are transient routing and
session facts. Peer pins, names, grants, and revocation remain authority facts.
Contract-only clients must not acquire network or cryptography dependencies.

The tray owns the supervisor through its existing Tokio runtime. Capture that
runtime at host construction; local socket workers do not have an ambient Tokio
runtime. Start networking only for an active authority. Inactive and shadow
hosts own no listener, discovery worker, or retry timer. Promotion follows the
existing explicit event and writer-acquisition rules. A network failure does
not regenerate identity, reset storage, or make local authority controls fail.

Stop requests remain bound to authority identity, activation, and revision.
If stopping cannot finish synchronously, report stopping and reject new starts
until completion. Stop and shutdown cancel all accept, dial, handshake, retry,
and session work, then observe task completion. Release every authority/config
clone before reporting the writer released. No host mutex is held across await,
and no synchronous request blocks a Tokio worker waiting on its own runtime.

One tracked owner supervises child tasks. Cancellation is not a completion
receipt, nor is dropping a JoinHandle. Owner drop must signal cancellation and
leave an owned reap path; ordinary stop and shutdown must observe it. Completion
callbacks carry the activation ID so an old owner cannot alter a replacement.
Partial startup failure closes every resource already acquired. A discovery
cleanup failure is visible and blocks an overlapping replacement until cleanup
is proven; it cannot be hidden behind a successful stop reply.

Cleanup acknowledgements have a bounded deadline. An unresponsive unregister
receipt must not prevent issuing shutdown. A missed deadline is unproven cleanup,
keeps replacement blocked, and produces a failed shutdown result instead of
making ordinary app exit wait forever. Inspect the selected daemon's status
receipt for an already stopped daemon; a failed command enqueue alone is not
proof either way. Never claim its worker was joined from a cancellation signal.

Direct exec and rolling-restart paths must await the same peer owner's shutdown
before process replacement. Use the existing runtime's attached handle. A new
global peer registry or a second shutdown controller is unnecessary. Cleanup
failure prevents exec and is reported through the existing restart failure path.
Artifact selection and verified staging continue through their current owners.

## Discovery and connections

Use mdns-sd 0.21.4 for `_qol-peer._tcp.local.` discovery. The advertisement
contains only protocol version and the claimed peer ID, with the normal TCP
port in the service record. It contains no invitation, name, key, grant, or
device inventory. Discovered claims never insert trust or establish online
state. Resolve pins from the same live authority at connection time.

This first adapter binds an ephemeral IPv4 TCP listener and advertises only
IPv4 addresses that it can serve. Keep this limit explicit in the network
contract and report IPv6 routing as unsupported in this round; do not strip
scope IDs into an incorrectly routable address. A later IPv6 adapter must use
the same supervisor and trust owner. Restrict production advertisements to
non-loopback interfaces; isolated tests supply loopback explicitly.

Retain at most eight endpoint hints for each currently linked peer, at most 256
peers total. Reject malformed IDs, unsupported versions, zero ports and
unspecified, multicast, or broadcast destinations. Ignore unknown identities
without allocating a durable record. Service removal invalidates that source's
hints, not an authenticated connection. Discovery failure invalidates its
hints and is separate from listener or session readiness. No second persisted
inventory or parallel address polling is introduced.

When a discovery event channel closes, disable that receive branch, invalidate
its hints, and expose discovery failure. Retain the owned discovery future until
it completes or its bounded close fails. A permanently ready closed-channel
branch must not spin or starve accepted TCP connections.

Bound in-flight handshakes/dials to 16 and accepted active sessions to one per
linked peer, at most 256. Check capacity before spawning work. TCP connect has a
five-second deadline; existing TLS and hello deadlines continue to apply.
Every accepted connection uses NormalSession::accept; every dial uses
NormalSession::connect. Start its run future immediately once admitted.

Simultaneous connections must converge deterministically. Prefer the connection
initiated by the smaller PeerId; resolve any remaining collision using the nonce
pair ordered by peer identity so both ends choose the same connection. Cancel
and reap losers. Keep only one outgoing attempt per peer at a time. An old
session's exit must compare the full generation before clearing a newer one.

Retry failed connections only while valid hints remain and no session is
active. Use bounded per-peer backoff, capped at 30 seconds, and do not reset it
on repeated advertisements or unrelated authority mutations. No plugin
operation is replayed by a connection retry. Dormant/inactive owners have no
periodic retry loop. Revocation or an authority fault cancels matching work and
removes its live projection promptly through the existing authority watch.

## Readiness and local projections

Expose listener, discovery, stopping, and failure state through the existing
typed PeerAdmin route. Keep a separate in-memory network revision for changing
session facts. It is neither a trust-store revision nor a new identity.
Session pages contain at most 16 entries and bind authority identity,
activation, store revision, and network revision. Stale pages fail instead of
silently mixing snapshots. Derive names from the authority projection.

Only successful mutual TLS and validated hello produce an authenticated-session
entry. The supervisor's live generation and task outcome own reachability.
Discovery announcements, old events, and successful socket writes cannot mark
a peer connected. Heartbeat/idle limits remain those in the normal-session
contract. An authenticated connection alone grants no plugin operation.

Normal sessions still carry only hello and heartbeat. Add no generic request,
payload, or plugin-dispatch variant before durable admission and replay handling.
The local CLI consumes canonical administration requests and responses and can
display additive network replies without owning another schema.

## Dependency evidence and testing

The library starts a worker thread and can report multicast errors after its
constructor returns. Observe monitor events; do not equate construction or
queued registration with announced readiness. Unregister and shutdown return
receivers for completion. Keep those acknowledgements in the owned close path.
[ServiceDaemon documentation](https://docs.rs/mdns-sd/latest/mdns_sd/struct.ServiceDaemon.html)

Automatic service address selection can be limited with `set_interfaces` and
`IfKind::IPv4`; configure the advertised family to match the listener.
[ServiceInfo documentation](https://docs.rs/mdns-sd/latest/mdns_sd/struct.ServiceInfo.html),
[interface selectors](https://docs.rs/mdns-sd/latest/mdns_sd/enum.IfKind.html).

Author deterministic supervisor tests with fake discovery and real generated
TLS over isolated loopback transports. Cover constructor/late discovery failure,
capacity, hint replacement/removal, simultaneous dial convergence, revoke during
handshake, full-generation stale exits, cancellation/reaping, and persistent
writer reacquisition after stop. Exercise real host requests and promotion with
an injected discovery adapter. Do not disable production networking globally
under cfg(test), and do not use host multicast, trust stores, or devices.

Guest verification remains required for real multicast and host lifecycle
behavior on each supported platform. A unit test or listening TCP port is not
evidence that the complete handoff works.
