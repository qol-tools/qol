# Core peer authority and storage

Status: selected implementation boundary; no production authority is active.
The [design](2026-09-27-linked-devices-design.md) owns delivery and migration.
The [transport contract](2026-09-27-peer-transport-v1.md) owns cryptography.

## Dependency and contract ownership

`qol-conventions::plugin_id` owns the shared PluginId and PluginUid value types.
`qol-conventions::operations` owns OperationKind, Invocation, OperationIdentity,
and OperationKey. Existing plugin-api paths remain facade re-exports. The
catalog and its declaration resolution stay in qol-plugin-api. Shared syntax
predicates move with their value contract and retain existing public paths and
behavior. No network crate imports plugin-api through a reverse runtime edge.

OperationKey has this JSON representation:

```json
{
  "identity": {"scope": "stable", "value": "immutable-plugin-uid"},
  "kind": "action",
  "name": "operation_name"
}
```

The local identity variant is represented explicitly as `scope: "local"` for
local catalog consumers. Peer grants reject it. Kind is one of action, query,
or stream. Wire parsing rejects unknown fields and tags; no label-to-UID
conversion exists. Operation syntax alone establishes no authorization.

## Authority lifecycle

`qol-peers`' service feature owns a cloneable handle to one locked state and one
storage writer. The host creates that authority; consumers use its API. TLS
TrustPolicy reads the same state through this handle. There is no separate
transport pin cache or independently writable settings model.

Provide separate explicit operations for an in-memory session, creating a new
persistent authority, and opening an existing persistent authority. Creation
requires a new authority directory; opening requires an existing valid snapshot.
Neither path silently replaces a missing, corrupt, unsupported, or mismatched
identity. Errors leave recovery decisions with the local user. Session mode
never writes credential or trust material. Persistent selection cannot arrive
through the remote operation parser.

A persistent authority holds a fixed-inode exclusive lock beneath its canonical
root for its entire lifetime. Do not unlink or replace the lockfile. Reject
symlink endpoints, nonregular files, unsafe ownership/permissions, and hardlinked
authority files. Canonical aliases cannot obtain two writers. The authority
directory is private to the local user. Platform-specific enforcement belongs
behind a capability-local platform facade. Platforms without verified private,
durable storage report unsupported for persistent mode; they must not silently
claim guarantees from no-op filesystem adapters.

## Snapshot and publication

One versioned `state.json` contains the local certificate/key, display name,
current peer pins and operation grants, tombstones, and store revision. Later
request and PointZ migration state extend that same snapshot and commit API;
they do not introduce another registry or commit marker file. Snapshot size is
bounded to 32 MiB, live peers to 256, grants per peer to 128, and retained
tombstones to 4096. Exhaustion returns an explicit error before mutation.

Use the existing qol-fs atomic private write primitive. Validate and serialize
the candidate, durably replace the snapshot, then publish it in memory. A
persistence error, including uncertain post-replacement sync failure, marks the
authority faulted and denies all trust/grant checks until reopening reconciles
the durable snapshot. Never acknowledge a successful mutation from an uncommitted
in-memory candidate. Private serialization buffers and key fields are redacted
and zeroized where owned; errors do not contain raw file contents or secrets.

Missing or duplicate pins, mismatched derived peer IDs, duplicate grants,
unknown schema versions, invalid secrets, and inconsistent active/tombstoned
records fail opening. Reopening a valid authority preserves identity and grants.
An expired local certificate may be renewed with its validated stored key before
publication: validate its profile, original signature, pin, and key match, issue
a current certificate for the same key, and durably save it first. This local
recovery path never makes an expired remote certificate acceptable.

## Mutations and grants

All mutations use the same store revision, encoded as a canonical decimal
string in the public wire contract. Expected-revision checks reject lost
updates. Increment with checked arithmetic. Public projections contain the
local peer ID, names, lifetime, revision, and peer/grant status, never private
keys. A projection is read-only data and cannot become a new writer.

Link insertion is an internal service operation reserved for completed enrollment
or controlled migration. Initial links have no grants. The network parser does
not expose administrative insertion, grant editing, persistence selection, or
revocation. Local administration changes exact canonical operation grants;
permissions do not expand by plugin name, wildcard, or future catalog additions.

The foundation exposes `has_grant` as a permission fact only. Actual dispatch
also requires the current canonical catalog, explicit exposure, availability,
fresh session, and domain preconditions. Optional resource constraints will use
the owning domain's validated contract; no arbitrary policy language or guessed
argument interpretation is introduced by storage.

Revocation immediately closes the authority's trust/admission gate for that
principal, commits its tombstone and removal of grants, then acknowledges.
Commit failure leaves the entire authority faulted. Existing sessions are
closed by the later session manager using this same mutation result. Automatic
reimport or ordinary link insertion cannot remove a tombstone. Explicit relinking
needs a new locally approved enrollment and a dedicated transition.

## Foundation round and evidence

This round implements lifecycle, snapshot persistence, projections, exact grants,
revocation, and live TLS trust lookup. It does not activate a host listener or
implement enrollment, request execution, PointZ migration, or Bluetooth handoff.
The full feature remains incomplete until those consumers and workflows land.

Tests use isolated directories and generated credentials. Required cases include
competing writers in another process, canonical path aliases, restart durability,
corrupt/missing/oversized/duplicate state, unsafe files, failed commits, stale
revisions, revocation and insertion after revocation, and session mode without
disk credentials. A TLS handshake through the authority's live TrustPolicy
must observe revocation. No test uses host trust data or real peripherals.
