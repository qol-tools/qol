# Deterministic sessions verification

## Outcome and sequence

Replace repeated agent formatting rounds and hand-parsed check logs with repository-owned operations and evidence. Phase one extends existing qol check. Phase two integrates explicit verification requests into session completion once capability work is integrated. No duplicate CI runner, arbitrary shell hooks, auto-acceptance, retry loops, model/budget changes, or test-free pass. The canonical skill policy is committed in qol-skills before implementing this improvement.

## Phase one ownership

One implementation lane owns tools/qol-cli/src/commands/check/ recursively, tools/qol-cli/src/cli/contract.rs only check entries, tools/qol-cli/src/cli/mod.rs only check help tests, and docs/notes/sessions-verification.md. Root owns this plan. No sessions module, manifests, lockfiles, CI, unrelated skills, or installed binaries changed. Add no dependencies. Existing check implementation owns planning, process containment, cancellation, and reports; extend it rather than adding another runner.

## Command contract

Preserve existing qol check, --staged, --lint behavior unless flags requested. Add --base REV (resolve to a commit once, use existing affected planner), --report PATH (atomic copy/publication of authoritative report at requested location, retaining normal logs/run directory), repeatable --format-owned PATH (only worktree full mode; reject alongside staged/lint). Paths must be exact repository-relative Rust source regular files, not globs/directories/symlinks, not outside root, deduplicate. Validate every requested file and formatter prerequisite before any write. No implicit whole-workspace write and no module-recursive formatting. Use discovered workspace/crate edition and owning rustfmt configuration. Use argv-safe subprocess calls. Formatter failure retains evidence and fails the run; do not hand-rewrite source or silently stage changes. Record formatted paths and actual before/after hashes.

## Source identity and concurrency

Fingerprint check input using HEAD, index state plus tracked/untracked/deleted file paths, types/modes and bytes relevant to the source tree; exclude ignored build outputs. Handle spaces/newlines/renames using NUL-safe Git output and do not follow symlinks outside root. Full tracked source hashing is acceptable initially; never truncate and claim pass. Include untracked source and deletion. Report fingerprint after allowed formatting and before checks; recapture after checks. A mismatch marks the result stale with nonzero exit, not pass. Report path inside tree must not itself cause staleness: exclude only exact node-owned artifacts and usual ignored outputs, never broad user paths. Unsupported input states must return explicit failure/skipped evidence, not verified pass.

Reuse existing file-lock/containment owner for serializing check commands against the resolved effective Cargo target directory (including CARGO_TARGET_DIR). Preserve staged private target isolation. Bounded cancellation-aware locking with explicit contention result is acceptable; no unbounded sleep or new daemon. Formatter ownership alone does not prove writers are idle: phase one is architect-invoked after fan-in; document that requirement. Phase two will enforce round quiescence before automatic invocation.

## Reports

Extend existing report.json additively with requested/resolved base, source fingerprints before/after checks, formatted files/hashes, actual argv/exit code per step (preserve existing duration/status), and explicit stale/cancelled/error state. --report path must receive a failure report for validation/runtime failures whenever safely possible. Never label lint-only as full verification. Full checks retain UI/release checks and affected build, strict clippy, tests/doctests with --locked. Reuse current execution engine. Do not expand test scope to unrelated historical commits when --base HEAD was explicitly selected.

## Acceptance

Tests should cover parser compatibility/conflicts, explicit base overriding default, safe owned formatting including parent module sibling untouched, rejected traversal/symlink/glob paths before writes, filename spaces/newlines, stale source changes, added/deleted/untracked content, same-target lock contention/cancellation, report publication on success/failure and read-only lint not becoming full pass. Use fake runner/isolated temporary Git repositories. Real central gate: scoped cargo build/test/clippy --locked, formatter check, repository check with explicit --base HEAD, plus literal temporary fixture invocation of new command and machine report inspection. Lanes only read/edit owned files and report; no build/test/lint/format/git commands. Root may run deterministic formatters under the newly committed skill policy.

## Deferred phase two (still required for overall user request)

Wire typed verification request and owned path list into sessions round metadata; one verification after grouped writers finish, verified source evidence in wake/next/receipt, safe cancellation and retry-free failure. Never infer hook commands or ownership from agent prose. Use the check node above. Do not implement phase two in this lane.

## Phase one correction round 1

Root gate: build and strict lint pass, 1266 tests pass, four fail (three rustfmt config failures and excluded report fingerprint). Logs /tmp/sessions-verification-test-r1.log, /tmp/sessions-verification-clippy-r1.log. Root has mechanically formatted owned Rust files; do not undo whitespace. Read reviewer evidence /home/kmrh47/.local/share/qol-tray/sessions/groups/sessions-verification-review/rounds/1/combined.md, but instructions below own the fix scope.

1. Formatter config discovery: find actual nearest rustfmt.toml/.rustfmt.toml via ancestor traversal, supply that file only when present. No --config-path directory without a config. Preserve stdin-only formatting and edition handling. Existing three config tests must pass, including no-config repositories.
2. Report destination validation applies on normal and parse-error publication. Reject tracked source paths, symlinks (including unsafe ancestor traversal), directories, and existing files unless they are an authentic prior qol-check report of expected schema. Never overwrite source on invalid arguments. Reject missing/dash-prefixed --report values so --report --lint cannot swallow the mode. Do not exclude caller-chosen report paths from source fingerprint: publication happens after recapture; validate destination separately. Adapt old exclusion test to this intended behavior, retaining added-report/source change detection. External or ignored new report files remain supported.
3. Target exclusions must never hide tracked source. Reject effective target equal to repository root or in-tree targets containing tracked files; only exclude known node-owned default targets or git-ignored safe target locations. Apply correctly for full/lint/staged modes. Resolve canonical target identity consistently for locking and exclusions; retain explicit errors for unsupported cases. Test source dir and root target refusal plus normal ignored and external targets.
4. Retain completed formatting records on partial failure, including path/hashes for every actual rewrite, and surface the failing file. Do not roll back or silently discard successful write evidence. Prevalidate paths/configs before mutation. Immediately before each write compare current bytes and file identity/path safety against the input read; if changed, fail without overwriting, record concurrent-edit evidence. Quiescence remains required; do not claim this removes all external writer races.
5. Route formatter subprocess through existing cancellation/containment owner; thread CancellationToken, check before each file and before write, bounded shutdown of child tree, no bare indefinite wait. Use existing process facilities; do not add a second general runner or dependencies. Preserve stdout input/output formatting semantics with bounded capture. Cancellation must prevent later files from being written.
6. Publication failures must be in the authoritative run-directory report, not only stderr/exit code. Do not say published_report exists or report state=verified if publication failed. Record publication state/error and rewrite final internal report coherently. A successful external copy must match final internal evidence. Test unwritable destination deterministically with injected failure rather than permission bits that root may bypass.
7. Test new failure paths with temp fixtures/injected formatter or existing runner seams. No implementation-mirroring tests replacing behavior. No source comments. Root will mechanically format and run verification; lane remains edit-only.

Ownership unchanged: check/ recursively, CLI check entries/help tests only, docs/notes/sessions-verification.md. No other files, manifests, sessions code, host config or plan edits. Report explicit remaining gaps; do not mark complete if requirements consciously omitted.

## Phase one correction round 2

Root mechanically formatted owned files. Central compile errors in /tmp/sessions-verification-test-r2.log and /tmp/sessions-verification-clippy-r2.log: FormatFailure lacks Debug for unwrap test callers; resolve_check_target moves canonical into path and then reuses it for exclusion in two branches. Fix these without weakening tests, cloning owned paths where necessary.

One report-truth issue remains: publish_and_finalize writes an internal verified/published report before external publication actually succeeds. Use an explicit pending publication state in the durable internal report until the external rename succeeds. Then finalize published state and rewrite authoritative evidence; ensure failure/cancellation cannot leave the authoritative report claiming successful publication. Successful final internal/external reports should converge to identical bytes, but an interrupted intermediate state must remain pending/unknown, not published. Use injected publication callbacks to assert internal state observed during the attempt is not published. No stronger cross-file atomicity claim than the implementation provides; document recoverable intermediate states. Retain injected failure tests.

Ownership and edit-only prohibitions unchanged. Root handles formatting; no format-only fixes delegated. Report changed files/lines and gaps only.

## Phase one correction round 3

Central gate: build and clippy pass. Tests: 1297 passed, one failed (fingerprint_excludes_only_named_node_owned_paths), see /tmp/sessions-verification-test-r3.log. Root traced the bug: hash_record adds kind/path before hash_content notices the exclusion. Creation/deletion/rename of untracked node-owned artifacts therefore changes the fingerprint even though their bytes are excluded. Skip the entire untracked record for approved node-owned exclusions before hashing any field. Never hide tracked source records or tracked bytes under exclusions: preserve tracked file identity even if a caller accidentally supplies a broad exclusion. Add behavior coverage for creation, mutation, deletion of excluded untracked artifacts and tracked edits under an exclusion. Preserve ordinary untracked source and caller-chosen report visibility. Do not weaken the existing failing test or expand exclusions.

Same edit-only ownership/prohibitions. Root handles formatting and central verification. Report changed lines and any gaps.
