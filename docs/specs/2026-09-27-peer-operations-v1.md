# Core remote operations, first integrated slice

Status: selected intermediate implementation. This slice must execute an explicitly exposed operation between two isolated attached hosts through the existing canonical local dispatcher. PointZ cutover, Bluetooth coordination and handoff, settings, guest evidence and the complete delivery gate remain required before release.

## Existing owners and scope

qol-peers owns peer identity, trust, grants, authenticated sessions and durable request outcomes. Its existing authority snapshot and exclusive writer own the operation ledger. The tray supplies the installed canonical operation catalog and existing plugin executor. Runtime and CLI expose the same headless contract. No second peer registry, persisted operation catalog, dispatcher, listener or database.

This first execution class is short actions/queries on an already-running compatible daemon. Runtime-only execution, remote daemon startup, settings activation, streams and high-frequency input are unsupported in this slice. Local dispatch retains its existing supported classes. Peer exposure still comes only from the canonical manifest/runtime declaration, with empty grants by default.

## Request identity and bytes

A request names the authenticated receiving PeerId, stable OperationKey, receiver-issued epoch, monotonically increasing sequence, random 128-bit request ID, immutable relative timeout and bounded argument document. Decimal-string u64 and base64url identifier conventions reuse existing contract owners. No caller-supplied peer identity, argv, local role or plugin directory.

Select exact-byte body identity instead of RFC8785/JCS. The sender freezes the UTF-8 operation-body document before allocation and preserves those bytes in the outbox. The wire carries that bounded document with an unambiguous encoding. SHA-256 covers the versioned complete immutable body, including key, arguments and original timeout. The receiver recomputes it. Different bytes for an already-used sequence are a conflict; no semantic canonicalization is claimed. Strict duplicate-key/UTF-8/depth/value validation applies to the nested document before decoding.

First-slice arguments are null or an object, at most 2048 encoded bytes. Use the existing strict JSON scanner to enforce strings, booleans, null, bounded containers and exact integers within signed 53-bit range; reject fractions, exponents, negative zero and out-of-range integer tokens before lossy Value conversion. This is a versioned bounded core message profile. The existing local domain handler remains responsible for operation-specific argument validation; do not invent a parallel generic input-schema language from today's descriptive annotation strings.

The receiver binds the admitted request to a digest of the actual canonical executable declaration selected from the catalog. Derive that digest in the shared catalog owner from its existing declaration data, not a new registry. It is receiver-selected: callers need not know remote filesystem/artifact details or provide a claimed schema digest. Recompute at dispatch; a changed declaration, exposure, UID resolution or selected plugin generation yields no-dispatch refusal. Freeze digest encoding fixtures without claiming JCS semantics.

## Durable transitions and bounds

Extend the existing authority snapshot to version 4. Opening v3 explicitly preserves identity, pins, grants, tombstones and enrollment transactions while initializing each existing link's operation epoch and empty ledger; commit before activation. New links initialize an epoch in the same link commit. No reconnect resets epochs or counters. No new third-party dependency.

The same authority writer persists:
- Per-link inbound epoch and admitted-through watermark.
- Sender target epoch, checked next sequence and immutable pending outbox entry, atomically before any send.
- Receiver request identity/body digest, selected declaration digest and Accepted state, atomically with advancing the watermark.
- DispatchStarted before crossing any possible local operation side-effect boundary.
- A bounded terminal result, no-dispatch refusal/cancellation, or Unknown before reporting it remotely.

Only the next sequence admits new work. Exact duplicates return original status/result, never acquire another dispatch owner. Different ID/body at a retained sequence conflicts. Any absent detail at or below the watermark is Expired, never executable. Gaps and old epochs refuse. Overflow refuses instead of wrapping.

Maintain at most one unresolved sender allocation per peer. A refusal before admission or lost admission reply does not discard its identity or allocate around the gap. Provide explicit reconcile/outcome and cancel for the original handle. Cancel for the next not-admitted sequence records a bounded cancelled receipt and advances the watermark without dispatch. A rejected next-sequence request that has been durably admitted gets its terminal receipt. If capacity prevents admission, it does not advance the watermark; explicit cancel must remain possible through reserved per-link metadata.

No automatic invocation replay, including PeerReplay::Idempotent. On receiver restart, persist Accepted -> CancelledBeforeDispatch and DispatchStarted -> Unknown before accepting sessions. Never reconstruct a live monotonic deadline or restart a handler. Sender restart preserves its original outbox and queries the remote outcome; it cannot silently allocate or send a replacement. A user-triggered initial send after preparation is separate from reconciliation; a send with uncertain delivery stays uncertain until a result/cancellation is established.

Known terminal outcomes may be queried by the same still-trusted originating peer for its own request after an operation grant is removed. Revocation blocks network access altogether. Completion cannot restore trust. Bounded in-flight settlement state can survive revoke until the owned worker finishes; then prune it with the retired link.

Use the researched bounded budget: 8 MiB encoded operation state within the existing 32 MiB total snapshot; up to 512 receiver records globally and 16 per peer; up to 256 unresolved sender records globally and one per peer. Argument documents max 2 KiB, result payloads max 4 KiB, entire encoded reserved record max 8 KiB. Reserve terminal capacity at admission. All authority mutations respect reservations and preserve revocation headroom. Count actual encoded bytes, including escaping/encoding and metadata; session-only storage obeys the same logical bounds.

Prune terminal detail deterministically while preserving watermarks. Never evict active work, unresolved sender allocation or live safety evidence to admit another operation. Return typed capacity/busy before effects. Keep a bounded compact sender history for inspection; pruning it cannot reset allocation counters.

## Authorization and dispatch boundary

Select durable DispatchStarted as the authorization linearization point. Before it commits, revocation, grant removal, stale session, stopping authority, removed/ambiguous plugin, changed declaration or changed daemon generation prevents dispatch. After that commit, cancellation/revocation cannot promise undo; one already-authorized attempt may complete. State this in the API. Do not claim physical first-write revocation.

Preserve lock order host -> plugin manager -> authority. The final selected declaration/generation and live exact grant are checked under the same ordered preparation/start guard. Storage must publish DispatchStarted before any action request write. Existing before_replace callbacks run too early and must not send operation bytes. Release manager/authority locks before waiting for I/O. No lock held across await, readiness loop or plugin callback.

The catalog owner captures immutable manager inputs under the manager guard and resolves filesystem-backed runtime declarations and executable availability after that guard is released. This is a request-local snapshot, never a second registry or persisted catalog. Remote preparation and revalidation reuse that owner; the existing installed-catalog entry point delegates to it so MCP and peer dispatch retain the same declaration rules. External catalog/artifact resolution and readiness happen outside host/manager locks. The final ordered guard compares the captured selection with current in-memory manager/lifecycle facts, including UID ambiguity, manifest, path/source, instance, endpoint and captured spawn artifact. The authority's own atomic persistence remains inside its existing mutation guard; it is the selected durable authorization point. This lock rule does not claim atomic protection from arbitrary external file edits after the latest external verification.

Extend the existing executor preparation seam with a checked remote policy and immutable selection. Do not perform an unrelated catalog check then call today's string-based executor, which resolves again and permits fallback. Reuse local execution/transport owners and canonical handlers; remote policy disallows startup, runtime fallback and reissue after dispatch starts.

Bind the local daemon request to the expected daemon incarnation through the shared runtime/daemon protocol. The upgraded shared daemon boundary validates the token before invoking any plugin parser, including theme/control interception. Advertise support only when that boundary is active. Use the existing side-effect-free readiness mechanism for capability/instance discovery; older or unverified daemons return update-required/unavailable and never receive an operation. A PID/path match alone cannot fence inherited listeners. The token is a local lifetime fence, not network authentication or a new plugin trust authority.

Reuse the manager's incarnation and artifact owners; if a host-unique boot component is necessary, define it once in the existing lifecycle owner. Verify the selected artifact using qol-artifact's existing identity policy. No duplicated artifact inspection or path-as-identity shortcut. Generation changes invalidate prepared selections. A replacement at the same inherited listener rejects a stale token.

Correct transport certainty in the existing local owner: distinguish NotSent before any attempted request write from OutcomeUnknown afterward, and retain explicit daemon replies. Bound serialization and response reads with the existing runtime message limit. A lost/malformed/oversized reply, timeout or partial write cannot trigger runtime fallback or query retry. Apply that certainty correction to existing local callers as well; do not keep a known duplicate-execution escape path. A handler error/fallback after dispatch does not prove zero side effects. Handled with no payload remains a handler acknowledgement, not device success; domain operation handles remain opaque to core.

## Sessions, host and client integration

Keep the existing normal TLS/hello identity and nonce contract. Exchange the durable receiver operation epoch in a new strict generation-bound control frame after hello. Operations require that frame for the elected session. Discovery and enrollment endpoints do not accept invocation. Every invocation/result/outcome/cancel message carries the correct nonce pair and request identity.

Retain one persistent frame reader and one writer. Add bounded queues under the existing network supervisor, with at most 16 global dispatch workers, one active invocation per peer and 1 MiB total queued message bytes. Control traffic has priority; no partial read/write restarts on unrelated select branches. Keep the existing heartbeat rules and add an explicit bounded operation rate, initially 32 inbound operation/control messages per peer per second with no unbounded burst queue.

Session replacement/disconnect invalidates not-started work for that generation. Started work remains owned long enough to settle its durable result; stale completion cannot publish current-session state. Reconnection may query the old durable request through its new session. No detached handler threads. If using spawn_blocking, retain and join the actual closure; aborting an awaiter is not termination. Shutdown stops admission, cancels queued work, awaits started work with its real I/O deadline and retains the authority writer on cleanup failure.

Request timeout defaults to 10 seconds and is capped at 10 seconds in this first slice. Receiver fixes the monotonic deadline at first admission; duplicates do not extend it. Long domain workflows return a short opaque handle and use later status queries.

Add canonical contract-only operation controls through the existing runtime socket and PlatformStateClient; preserve same-user credentials and activation-bound local mutations. No peer transport route may invoke PeerAdmin. Extend qol peers with thin invoke, outcome/reconcile, cancel and bounded request listing commands. Input comes from bounded stdin; commands and traces never print raw arguments as diagnostics. A local lost reply is Unknown, with request listing available to recover the original handle.

## Required integrated evidence

Author a real two-host generated-TLS test using the actual local API, canonical catalog, prepared executor and shared daemon listener. The deterministic fixture daemon counts invocations and exposes barriers without touching OS devices. Use existing public enrollment/discovery fixtures; factor shared test helpers instead of importing private symbols. A counted closure replacing the dispatcher is insufficient.

Prove:
- Empty grants, absent exposure, local/duplicate UID and invalid body execute nothing.
- One exposed granted request returns a durable result through the real dispatcher.
- Concurrent/matching duplicates execute once; conflicts, old epochs, pruned outcomes and gaps never execute.
- Lost network reply reconciles the stored result without another execution.
- Daemon mutation followed by reply loss remains Unknown, with runtime fallback and query retry counters zero.
- Receiver restart around admission/dispatch/result preserves the selected transitions and identity; sender restart preserves the outbox.
- Revoke/grant removal, contract changes and daemon replacement before DispatchStarted execute nothing; after-start cancellation makes no rollback claim.
- A replacement on an inherited listener rejects the old token before its parser.
- Bounded capacity/reservations, cancel-next sequence continuity, fragmented traffic/heartbeat and shutdown hold the declared bounds and writer ownership.

Use focused model/table tests for ledger transitions and existing fault-injection seams. Keep all external effects synthetic. The architect runs build, strict lint, selected tests and the eventual complete/guest gates centrally. No installation, actual Bluetooth release, PointZ migration or production exposure in this intermediate slice.
