# Linked computers implementation

Current checkpoint: the core peer authority, authenticated transport, canonical
operation dispatch, CLI, and native/web settings are implemented. After round16,
`qol check --base 987dc5aa0` passes the Linux build, strict lint, formatting,
7,684 Rust tests (six skipped), 622 UI tests and the repository script checks.
The supplemental service suite passes 214 tests. Evidence is retained in
`target/linked-computers-implementation/checkpoint-verified-checks/qol-check.json`
and `checkpoint-final-checks/peer-service/report.json` in the same directory family.

The settings controls use the same authority for names, pairing, revocation and
explicit grants. Operation choices derive from the canonical installed catalog;
unavailable declarations and uncertain mutation outcomes remain visible. The
architecture viewer derives its status and ownership tables from the design,
covering all 18 plugins, 39 shared crates and 84 ownership entries.

The user requested committing and pushing the completed checkpoint before more
feature work. Delivery stays on the paired `linked-computers` worktree branches.
PointZ compatibility and authority migration, shared Bluetooth/Controllers
mutation ownership, earbud handoff, and guest/multi-PC/hardware verification
remain required for the complete feature. No implementation has been installed
or released, and the viewer's handoff walkthrough remains illustrative.

The [reviewed design](../specs/2026-09-27-linked-computers-design.md) owns the architecture and acceptance criteria. This checkpoint preserves the work in progress; complete feature acceptance still includes the core peer service, PointZ cutover and Bluetooth workflow.

## Delivery sequence

1. Normalize plugin operations in the shared contract owner. Add explicit peer exposure and retry semantics to canonical declarations, defaulting to no peer access. Adapt MCP as the first real consumer while preserving its existing caller identity rules. Select the transport, pairing protocol, and migration mechanics from source evidence before network implementation.
2. Implement the headless peer authority with exclusive storage ownership, host lifecycle integration, local client operations, authenticated sessions, grants, request outcomes, and fresh projections. Migrate PointZ identity, pairing, discovery, and authentication in the same delivery; prove the legacy writer cannot coexist or resurrect revoked records.
3. Coordinate Bluetooth and Controllers connection mutations through their shared domain owner. Implement the handoff state machine, expiring reconnect holds, restart recovery, and verified device results through core requests.
4. Connect core settings and headless controls to the same APIs. Verify pairing, permission changes, reconnection, migration interruption, and handoff failures in isolated guests. Update the ownership design and skills to reflect actual implementation.

## Initial round

For this implementation, the user requires Codex GPT-6 Astra with High reasoning for every delegated round. Explicit user selection overrides the Sessions skill's default tier binding. The interrupted catalog draft remains unverified and must be reviewed before continuation.

The operation-catalog lane owns only shared declaration/catalog files and their MCP consumer. The protocol/migration scout owns only its report. Neither runs builds or edits the other's files. The architect reviews both outputs, resolves the protocol decision, and runs verification centrally before assigning integration work.

The catalog belongs in `qol-plugin-api`, which already depends on `qol-config`. Metadata shared with runtime declarations belongs in `qol-config`; no reverse dependency is introduced. The tray catalog adapter derives from installed validated contracts and current availability. It is not a new persisted registry. Stable plugin UID, operation kind, and operation name identify an operation. Local compatibility fallbacks must never create a remote trust identity from a mutable label.

## Verification workflow

The initial catalog round passed `qol check --base 987dc5aa0` with explicitly
owned formatting paths on Linux. Its report is
`/tmp/qol-linked-computers-catalog-check.json`, with a matching source fingerprint.
Subsequent source review found shipped CLI aliases and cross-kind names that
the new catalog rejects. That round is not accepted until a complete shipped
contract corpus regression and the compatibility corrections pass.

The next disjoint rounds correct the catalog and implement only the identity/TLS
foundation from the [transport contract](../specs/2026-09-27-peer-transport-v1.md).
The new crate remains an uncommitted intermediate until it has its actual host
and migrated PointZ consumers. No persistent authority or listener activates
during these rounds.

The catalog correction subsequently passed the full gate, including every
shipped contract and 7348 Nextest tests (six skipped). The service-feature TLS
suite compiled and passed twenty of twenty-one tests across unit/integration
runs; its TLS1.2 fixture and two lint expressions require correction. A source
review also identified same-key local renewal after certificate expiry as a
necessary storage integration case. These observations are recorded in
`target/linked-computers-implementation/round2-review.json`.

The next round follows the
[authority storage contract](../specs/2026-09-27-peer-authority-storage.md): shared
reference types, the single state/writer, and disjoint crypto corrections.
Supplemental opt-in service checks run through
`node target/linked-computers-implementation/verify-peer-service.mjs`; it records
exact argv, exits, logs, and source fingerprints in its report.json. This scoped
verification node supplements the repository gate and will feed the final guest
workflow rather than replace it.

The authority round passed source review and the complete repository gate at
`/tmp/qol-linked-computers-round3-retry-check.json`: 7357 Nextest tests passed,
six skipped, with matching fingerprint
`2c0434ab0152d4808c690c770556ea37590b9dcb5acf24955de2ef835003a345`.
The first attempt finished compilation but the check runner rejected residual
process-tree membership. The captured retry passed without source changes;
this does not establish that the runner issue is fixed. The supplemental peer
service build, strict Clippy, and all 51 tests also passed. Evidence is retained
in `target/linked-computers-implementation/round3-review.json` and
`peer-service-check/report.json`.

The next disjoint round implements the
[enrollment contract](../specs/2026-09-27-peer-enrollment-v1.md), the real host
lifecycle and local administration adapter, and bounded JSON framing. The peer
authority lane alone changes the shared state, snapshot, and peer crate facades.
The host lane authors local administration contracts and their actual runtime
consumer. The framing lane owns only its private codec module. Enrollment never
decodes local administration requests. Network listeners, ordinary peer request
execution, PointZ cutover, and Bluetooth remain later integration work; this
round does not authorize a partial release.

The enrollment/host/framing round compiled, but its central check did not
pass. The workspace runner again rejected residual process-tree membership
after Cargo finished compilation. The supplemental service suite passed 99
tests and failed three: extra fields were accepted on fieldless local and
enrollment variants, and a revocation test dropped its connection before
asserting a delivered response. Strict Clippy also rejected the unused normal
frame mode. Source review found that local mutation revisions need authority
identity and activation binding to reject delayed commands after replacement.
Evidence is retained in `target/linked-computers-implementation/round4-review.json`.

The next disjoint round corrects enrollment strictness and failure evidence,
binds local controls to the authority activation, and implements the
[normal-session mechanics](../specs/2026-09-27-peer-normal-session-v1.md) over the
same authority and framing codec. These remain intermediate components. No
implementation commit or feature acceptance is justified by this round alone.

The correction round passed the peer service build, strict lint, and all 145
tests, plus 31 focused host/runtime administration tests. Source review verified
strict fieldless variants, activation-bound mutations, expired reservation
handling, and normal sessions using the same live authority. The full workspace
build and formatting passed, but strict lint stopped on one pagination-test
expression; workspace tests therefore did not run. Round5 evidence is retained
in `target/linked-computers-implementation/round5-review.json`. No process-tree
cleanup failure recurred in this attempt, which does not prove that runner
issue fixed.

The next disjoint round adds explicit durable abandon/resume transitions for
outbound enrollment, a thin `qol peers` client through the existing local route,
and the [core network owner](../specs/2026-09-27-peer-network-v1.md) with real tray
lifecycle integration. The CLI lane also owns the one-line pagination lint
correction. The network lane owns additive local network projections; the CLI
serializes canonical response types so it does not duplicate their schema.
Every lane remains edit-only. The architect resolves manifest changes and runs
the central gate after all three finish. Full enrollment controls, request
admission, PointZ, Bluetooth, settings, and guest evidence remain outstanding.

Round6 added explicit outbound states in snapshot version 3, the canonical CLI,
and a core network supervisor with real host ownership. Architect source review
found no new outbound-lifecycle or CLI defect; execution of the new authority
tests remains blocked by network-test compilation. The CLI independently passed
strict lint and 21 focused tests. Dependency resolution succeeded for mdns-sd
0.21.4. The complete gate stopped on two extra closing angle brackets in host
handle types; the service build stopped on a partially moved network fixture.

Source review also found an unbounded discovery cleanup wait, a closed-event
channel that could spin, and direct exec restart paths missing peer teardown.
The next two edit-only lanes correct networking and integrate restart teardown,
with disjoint files and the same Codex Astra High requirement. Evidence lives
in `target/linked-computers-implementation/round6-review.json`; no complete gate,
runtime deployment, or feature acceptance is claimed for this round.

Round7 passed the supplemental peer service build, strict Clippy, and all 184
tests. The explicit dev-feature host/runtime check also passed strict Clippy
and all 57 selected tests. Source review verified bounded discovery completion,
closed-channel handling, writer retention on failed cleanup, and both restart
routes awaiting the existing attached core owner. Evidence is retained in
`target/linked-computers-implementation/round7-review.json`.

The full repository gate passed source guards, UI/release tests and formatting.
Cargo printed a completed workspace build, but the command runner again rejected
residual owned process-tree membership, so that gate did not reach lint or
workspace tests. No descendant identity or verified cause was captured; a
focused source investigation must identify the next diagnostic or correction
without weakening containment or counting repeated retries as a fix.

The next implementation round connects
[enrollment controls and TCP exchanges](../specs/2026-09-27-peer-enrollment-host-v1.md)
through the existing authority, network owner, attached host and CLI. Two
disjoint read-only lanes investigate the check-runner blocker and map every
Bluetooth/Controllers mutation into a concrete shared ownership proposal.
All lanes use Codex Astra High and remain edit-only. Reports are reviewed before
central execution. Full workspace acceptance, remote operation admission,
PointZ cutover, Bluetooth handoff, settings and guest evidence remain required.

Round8 connected enrollment through the existing core authority, network owner,
host socket and CLI. Strict CLI lint and 23 focused tests passed. The service
and dev-host integration suites could not compile because two fake discovery
fixtures imported a private cancellation helper. The full gate passed source
guards, UI/release tests and formatting, then failed the same compilation. Its
runner again reported residual process-tree membership and omitted Cargo's raw
exit status. Neither pairing integration nor the complete gate is accepted.
Evidence is in `target/linked-computers-implementation/round8-review.json`.

Source research confirmed that the check report lacks both the actual selected
containment backend and independent leader status. The next round adds bounded
failure evidence through the existing runner and process owner, preserving the
failed verdict and cleanup policy. The historical cause remains unknown; no
grace period, containment bypass or retry-until-green is selected.

The Bluetooth inventory found connection writers in audio repair, startup,
reconnect, Controllers reclaim and adapter maintenance. The shared owner must
coordinate all conflicting paths. Hardware state stays authoritative in the OS;
Bluetooth retains handoff policy and Controllers retains HID policy. Research
suggestions about timing constants, private device identity and broader Sound
routing changes remain unselected until the implementation contract is written.

Round9 has disjoint edit-only lanes correcting the two enrollment fixtures,
adding check failure evidence, and mapping the existing dispatcher/authority
seams for remote request admission. Every lane remains Codex Astra High. The
architect reviews source and verifies centrally after fan-in, including the
new secret-buffer and socket-I/O coverage. This is continued implementation,
with no partial release or final acceptance.

Round9 corrected the fixture imports and passed the current peer-service build,
strict lint and all 192 tests. The dev host/runtime gate passed strict lint and
74 tests, including two attached hosts pairing through the real local client
and generated TLS, explicit rejection/cancellation, and persistent recovery
after a lost reply. These are verified isolated test instances, not physical-PC
or multicast evidence.

The bounded check-runner evidence changes also passed strict CLI/process lint
and 76 selected tests, including preserved raw exit status on a residual
failure, unchanged failed verdict, owned cleanup, bounded observation and
nested process ownership. The current reports retain source fingerprints in
`target/linked-computers-implementation/round9-review.json`. The historical
full-check failure's cause remains unproven; no full workspace pass is claimed.

The next round is one integrated implementation lane under the selected
[remote operation contract](../specs/2026-09-27-peer-operations-v1.md).
It connects the existing authenticated session and authority writer to the
canonical plugin executor, local client and CLI, with bounded durable outcomes,
instance fencing and no replay after uncertain dispatch. It includes an actual
two-host/daemon fixture. No additional research fan-out is assigned. PointZ
cutover, neutral Bluetooth coordination, handoff UI and final guest verification
remain necessary for the usable delivery.

Round10 authored the connected remote-operation path, but central review did not
accept it. Peer-service build passed with 210 tests passing and two failing:
the new control handle retained the authority writer after network completion,
and one enrollment assertion still expected snapshot version 3. Strict lint
also found a collapsible conditional. The actual two-host fixture could not
compile because it imported a private daemon-lifecycle module. A separate
CLI/catalog/daemon gate passed strict lint and 102 selected tests, including
the shared daemon's incarnation fence. Personal review also found artifact
file inspection under the host/manager locks. One bounded Astra High correction
lane addresses those findings; no second research fan-out is assigned. Exact
commands, fingerprints and failures are in
`target/linked-computers-implementation/round10-review.json`.

Round11 fixed the control-handle writer lifetime, private fixture import,
snapshot assertion and service lint. Central peer-service build, strict lint
and all 212 tests passed. The actual two-host remote operation fixture also
passed through local clients, generated TLS, canonical catalog, prepared
executor and the shared daemon, including duplicate suppression and recovery
after lost replies. The complete host selection ran 241 tests: 233 passed and
eight failed because the common populated-store fixture writes peers into a
v4 snapshot without its required operation metadata. Host strict lint exposed
two additional daemon-lifecycle style errors. This is isolated headless
evidence, with synthetic discovery and the explicit test artifact verifier;
no real Bluetooth or installed two-PC result is claimed.

The remaining catalog lock correction needs the canonical catalog owner to
separate manager capture from file-backed resolution and pure live comparison.
Its scope now includes that owner and its tests, together with the common
snapshot fixture and the two lint corrections. One Astra High lane implements
that bounded correction. Exact evidence and limitations are recorded in
`target/linked-computers-implementation/round11-review.json`. Earbud handoff,
PointZ cutover, shared Bluetooth ownership, UI and guest verification remain
outstanding.

Round12 passed strict host lint, the real two-host request/recovery fixture,
the new lock-availability and selection-change regression, and the catalog/MCP
regressions. Its selected test run has 269 passes and one failure: the activation
fixture still uses revision 1 for Revoke after SetGrants, although its migrated
starting revision is now 1. The correction derives that expected revision from
the fixture's current authority. Peer-service inputs retain the exact round11
fingerprint, so its 212-test pass remains applicable without a repeated run.
The catalog has one capture/resolution/selection owner; external file and
readiness work now runs outside the host/manager guards. This round's evidence
is `target/linked-computers-implementation/round12-review.json`.

One small round corrects the remaining fixture and adds an explicit before/current
view to the existing architecture page, generated from this design's single
implementation-status table. This makes the tested core work visible while
keeping unfinished PointZ/Bluetooth/UI/device work explicit. It is a status
viewer, not a replacement for implementing or verifying the handoff.

Use the existing `qol check` node for affected builds, formatting, lint, tests, and source guards. Extend the existing guest tooling with one reproducible linked-computers workflow once the actual integration steps are established. Its inputs are guest identities and fixtures; its output is `report.json` containing source/artifact identities, commands, request traces, observed state, and failed or unsupported steps. Dynamic process arguments use argv arrays.

Tests target authorization and state transitions: default denial, contract ambiguity, revocation, competing writers, migration interruption, stale generations, request deduplication, and failed handoff reconciliation. No test may use the host's real trust store or Bluetooth connections.

## Trace decision

Enrich `TRAY_MCP` for catalog resolution failures without logging arguments. Introduce a focused linked-computers trace when the service is integrated: pairing lifecycle, authorization decisions, session generations, request IDs, timeout/reconciliation outcomes, and migration phases. Bluetooth traces retain device policy and readiness details. Secrets, pairing material, raw payloads, and private paths never enter these traces.
