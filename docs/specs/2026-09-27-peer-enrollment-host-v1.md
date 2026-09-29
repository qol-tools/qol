# Core enrollment controls and owned TCP exchanges

Status: selected intermediate integration. The enrollment, authority, normal-session, and network contracts remain authoritative. Complete delivery also requires remote operation admission, PointZ cutover, Bluetooth coordination, settings, and guest evidence.

## Ownership and transport

Extend the existing active core network owner. A separate ephemeral IPv4 enrollment listener uses the existing enrollment TLS configuration and ALPN; normal listener traffic still passes exclusively through NormalSession. Neither listener grants an operation. There is no plugin-owned pairing port, second key/store, detached service, or alternate local administration route.

The listener starts only with an explicitly active authority, on the same configured bind address as the normal listener. Project its actual bound port and readiness separately from normal listener/discovery status. Keep invitations out of discovery. Invitation address inputs are local routing hints: at most eight literal IPv4 unicast addresses; the core supplies its actual enrollment port. Reject unspecified/multicast/broadcast addresses and unsupported IPv6. Loopback is permitted only by injected test options. Recovery accepts at most eight explicit literal socket endpoints, with the same checks. A caller's endpoint can never replace the stored inviter pin.

Own at most eight inbound enrollment exchanges and eight outbound attempts, one outbound attempt per transaction. Bound queued local commands as well. Enforce budgets before spawning or retaining work. Reuse existing five-second TLS/frame limits, add a five-second TCP-connect deadline and an overall bounded exchange lifetime. Stop, revoke, authority fault, or local abandonment must cancel affected work promptly. Cancel and join every task before network completion; retain the existing writer-holding failure behavior if cleanup is unproven. Do not allow enrollment failure to silently stop the unrelated normal listener.

## Local headless controls

Expose every action through canonical qol-peers admin Request/Response types, the existing attached PeerHostHandle, and RuntimeRequest::PeerAdmin. Contract-only clients remain free of service/network/crypto dependencies. Use a bounded secret-bearing string newtype with redacted Debug and zeroizing storage for invitation documents; explicit serialization is the only export. Move/reuse the existing ExportedInvitation owner rather than inventing a second token codec.

Provide create/cancel invitation, list authenticated pending approvals, approve/reject a presented EnrollmentRequestKey, prepare outbound join, redeem prepared transaction, recover original transaction using supplied endpoints, abandon/resume, and bounded outbound-state/result queries. Rejection cancels that invitation through the existing authority. Approval shows authenticated peer identity/name and both actual lifetimes, then commits empty grants using the existing CAS operation.

Every local mutation carries ExpectedAuthority. Check authority identity, activation and revision against the same locked authority before changing state or admitting work. Existing enrollment methods own validation, durable changes, and original transaction/pin checks. Add narrow authority APIs only where atomic checks or the original persisted pin must be exposed safely; no snapshot mirror.

Preparing a join is a separate durable local action, returning its original transaction identifier. Sending or recovering never prepares a replacement implicitly. Admission returns promptly with the original transaction and an explicit queued/running result; local socket workers never block awaiting human approval. A lost local admission reply is unknown and never automatically resent. Bounded runtime attempt state belongs to the active network owner; durable pending/abandoned/committed state belongs only to the authority. Do not call a queued or locally committed attempt bilaterally complete. Preserve completed/unknown/rejected outcomes from the existing exchange protocol. If runtime outcome was lost on restart, expose unavailable/unknown and recover the preserved original transaction explicitly.

Read projections are snapshots from their owner. Pending approvals are intrinsically bounded by eight invitations. Outbound pages use the authority activation and store revision with at most 16 entries. Transient attempt results must bind activation and be queried independently or with their own view revision; an authority revision alone cannot stabilize network progress. Do not silently evict unresolved active work or durable recovery evidence. Unknown transaction and exhausted runtime capacity are typed refusals. No endpoint or invitation secret is persisted in another registry.

## CLI and secret boundary

Add thin qol peers controls for this API. Invitations are read from bounded stdin, never argv, logs, automatic clipboard, or ordinary status output. Export a token only in the explicit create-invitation response. Reuse canonical Request/Response and identifier parsers. Convenience mutations read a stamp once and dispatch once, with no stale or unknown retries. Preserve raw request for automation and offline help.

Audit the existing local client serialization/read buffers, server accepted request storage and reply buffers, and CLI stdin/output paths for secret copies. Use zeroizing owned buffers where the API permits; errors/logs never echo payloads. Explicit user-facing stdout export is intentional. Avoid unbounded reads or broad changes to unrelated IPC consumers. The existing same-user credential boundary remains mandatory and is not a remote grant.

## Evidence

Use generated credentials, temporary stores and isolated loopback only. Exercise two real attached core hosts through their real local admin route: create invitation, prepare, queue redemption, inspect pending authenticated approval, approve, observe bilateral completion with empty grants. Also exercise lost reply/recovery after owner restart, cancellation/expiry, abandon during exchange, stale activation/revision, wrong pin and unsupported/bad endpoints, duplicate active admission, bounded capacity, shutdown/reaping and writer reacquisition. No host multicast, trust data, installed runtime, real earbuds, or profile changes.

Trace TRAY_PEERS with action/result, transaction IDs and activation where useful, never exported documents, secret bytes or raw request payloads. Author the focused tests; the architect reviews and verifies centrally after fan-in.
