# qol-memory store hygiene

Date: 2026-09-14
Branch: store-hygiene
Status: implementation spec, architect-authored

## Problem

The qol-memory store accumulated several categories of data beyond the memory
itself: 4 old full copies of the corpus kept as research fixtures plus 14 empty
directories from a buggy research cleanup, a dead 44 MB manual backup, 142
research experiment report directories and 56 MB of score dumps written into
the store by research script defaults, 5 orphaned crash-leftover temp files,
and three product logs with no caps (hook.log, feedback.jsonl,
continue.marker.json entries never dropped). The notes retention shipped in
4bd9cb591 fixed one accumulation site; this slice makes every byte in the
store justifiable and bounds every growth path.

Classification established by the store-audit scouts (2026-09-14, grouped
report under the sessions data dir):

- Primary: units.jsonl, retrievals.jsonl (capped 10M/1M), feedback.jsonl
  (uncapped), continue.marker.json (uncapped entries), ingest-state.json
- Bounded safety: notes runs (notes_runs_kept, default 7 live)
- Derived caches: idx-*.json (single generation per layer and caller slug,
  deleted by reindex), skills/index.json (product-read), answer-bindings
  (CACHE_LIMIT 256), verification/
- Research artifacts that must leave the store: snapshot/ fixtures, eval/,
  ingest.jsonl, ingest/ reports, skills/eval/, distinctive.json,
  qolmem-claims.json, manifest.json
- Dead: units.jsonl.bak-preprune (only unique content is the 32 refill-loop
  units deliberately removed on Sep 1), empty snapshot dirs, stale
  .ingest-state.json.*.tmp files

## Acceptance criteria

1. Product code caps every append-only store file it owns: hook.log and
   feedback.jsonl rotate with the same cap and tail semantics as
   retrievals.jsonl, and continue.marker.json drops entries not seen for 90
   days with a backfilled last_seen field.
2. A warm-start hygiene sweep removes crash-leftover temp files (store-root
   dotfiles ending .tmp and notes/.tmp-* directories) older than one hour,
   counts what it removed, and traces the event.
3. The retrieval-log rotation becomes crash-safe (atomic tail rewrite), with
   a test proving the tail survives interruption semantics.
4. Doctor gains a store hygiene check reporting stale temp files, oversized
   logs, and research artifacts still present in the store.
5. Research scripts stop writing into the memory store by default: all
   research outputs (snapshot corpora, eval, parity, skills-eval, ingest
   reports, candidates report, distinctive, manifest, walk ledger) default
   under the repo's gitignored reports/qol-memory/ tree; the store remains
   reachable only through explicit --store or QOL_MEMORY_STORE.
6. Pinned research fixtures move to reports/qol-memory/fixtures/ and every
   pinned-run reader resolves them there: snapshot pin 2026-08-10T21-38-02-273Z
   and 2026-08-12T18-46-58-129Z, notes pin 2026-08-13T16:31:40.844Z.
7. snapshot.mjs --keep removes the directories it empties, and eval.mjs keeps
   only its 10 newest run directories.
8. Tests cover the sweeper, caps, marker pruning, atomic rotation, and the
   updated research pin resolution; the architect relocates the live data
   (including extracting the notes pin from the archive tarball) and runs the
   node test scripts against the relocated fixtures, all before any
   destructive live-store step and with user approval for the destructive
   step.

## Lane 1, owned paths (product, Rust)

- plugins/memory/src/store/mod.rs
- plugins/memory/src/app/mod.rs
- plugins/memory/src/continue_recall/mod.rs
- plugins/memory/src/feedback.rs
- plugins/memory/src/retrieval_log/mod.rs
- plugins/memory/src/doctor/mod.rs

### Design

store/mod.rs, new method on Store:

```rust
pub fn sweep_stale_temp_files(&self, older_than: std::time::Duration) -> usize
```

Semantics: scan the store root for entries whose names match the atomic-write
temp shape (a leading dot and a .tmp suffix, the `.<name>.<6 alnum>.tmp`
pattern produced by qol_fs::atomic_write) and are files, plus notes/ children
named `.tmp-*` that are directories; remove those whose mtime is older than
the cutoff; never touch anything else; return the removed count. NotFound on
the root returns 0. The sweeper never acquires locks; one-hour-old temps are
crash residue, not in-flight writes.

app/mod.rs: extend the warm-start hygiene call site that currently runs
prune_notes_runs_at_warm (app/mod.rs:134 in the shipped code) to also call
the sweep and the marker prune; one probe on QOL_MEMORY_DAEMON:
`event=hygiene_sweep removed_tmp={n} pruned_markers={n}`. Failure paths
follow the existing stderr plus error-probe convention.

continue_recall/mod.rs: bound hook.log. Reuse the cap-and-tail rotation
semantics of retrieval_log; if retrieval_log's rotate helper can be
generalized inside the plugin without a third copy of the logic, share it,
otherwise mirror the exact semantics. Constant HOOK_LOG_CAP 10 MB, tail
1 MB, matching RETRIEVAL_LOG_CAP and RETRIEVAL_LOG_TAIL. Also extend
continue.marker.json handling: on write, backfill a last_seen timestamp for
entries that lack one; at warm start, drop entries whose last_seen is older
than 90 days and rewrite the file atomically. Entries are keyed by cwd today;
preserve that shape and only add the field.

feedback.rs: same cap-and-tail treatment for feedback.jsonl, constants
FEEDBACK_LOG_CAP 10 MB, tail 1 MB.

retrieval_log/mod.rs: make rotate_if_needed crash-safe: write the retained
tail through qol_fs::atomic_write instead of truncate-then-write, preserving
the cap and tail constants and the observable rotation behavior.

doctor/mod.rs: one new check, id store_hygiene, reporting: count of stale
temp files older than one hour, hook.log and feedback.jsonl sizes against
their caps, and whether snapshot/, eval/, or ingest/ research directories are
non-empty (message names relocation as the fix). Status ok when nothing is
stale or oversized and no research directories are populated.

Tests, in the owned files: sweeper removes only old temps and never touches
fresh ones or non-temp names (including the .distill-catchall.ts marker and
.distill.lock); hook.log rotation keeps the tail under cap; feedback rotation
same; marker prune backfills last_seen, drops only entries older than 90
days, and preserves recent entries; retrieval rotation leaves exactly the
tail bytes and survives a simulated partial write; doctor check renders ok
and warn paths.

## Lane 2, owned paths (research scripts)

- docs/research/qol-memory/lib/store-path.js
- docs/research/qol-memory/snapshot.mjs
- docs/research/qol-memory/notes.mjs
- docs/research/qol-memory/decisions.mjs
- docs/research/qol-memory/ask.mjs
- docs/research/qol-memory/ingest.mjs
- docs/research/qol-memory/candidates.mjs
- docs/research/qol-memory/parity.mjs
- docs/research/qol-memory/distinctive.mjs
- docs/research/qol-memory/eval/eval.mjs
- docs/research/qol-memory/eval/skills-eval.mjs
- docs/research/qol-memory/eval/verdict-eval.mjs
- docs/research/qol-memory/test-e2e.mjs
- docs/research/qol-memory/test-alias.mjs
- docs/research/qol-memory/test-units-replace.mjs
- docs/research/qol-memory/docs note: prepend a dated note to
  docs/research/qol-memory.md stating that outputs default under
  reports/qol-memory/ and fixtures live under reports/qol-memory/fixtures/,
  without rewriting the historical log below it

### Design

lib/store-path.js: add two resolvers alongside qolMemoryStore():
`researchOutputRoot()` returning `<repo>/reports/qol-memory` and
`fixturesRoot()` returning `<repo>/reports/qol-memory/fixtures`, both
resolved relative to the module location (docs/research/qol-memory is inside
the repo; reports/ is gitignored and already hosts 1.3 GB of research
output). --store and QOL_MEMORY_STORE keep their current meaning: the memory
store, used only for memory data.

Output redirections, default paths only, all overridable where a flag already
exists:

- snapshot.mjs: OUT_DIR default becomes researchOutputRoot()/snapshot;
  the walk ledger INGEST_PATH becomes researchOutputRoot()/ingest.jsonl
- eval/eval.mjs: run dir default becomes researchOutputRoot()/eval; add
  retention keeping the 10 newest run dirs under that root, removing older
  ones including their directories
- eval/skills-eval.mjs: researchOutputRoot()/skills-eval
- parity.mjs: researchOutputRoot()/parity
- ingest.mjs: reports become researchOutputRoot()/ingest-reports
- candidates.mjs: report.json becomes researchOutputRoot()/candidates-report.json
- ask.mjs: manifest.json becomes researchOutputRoot()/manifest.json
- distinctive.mjs: researchOutputRoot()/distinctive.json

Fixture resolution, replacing store-relative pinned reads:

- The snapshot pin 2026-08-10T21-38-02-273Z (eval/eval.mjs:18-27, notes.mjs,
  decisions.mjs snapshot mode, ingest.mjs, test-e2e.mjs) resolves from
  fixturesRoot()/snapshot/<pin>
- The snapshot pin 2026-08-12T18-46-58-129Z and the notes pin
  2026-08-13T16:31:40.844Z (test-alias.mjs, test-units-replace.mjs,
  eval/verdict-eval.mjs, candidates.mjs PINNED_NOTES baseline) resolve from
  fixturesRoot()/snapshot/<pin> and fixturesRoot()/notes/<pin>

snapshot.mjs --keep: after unlinking files in expired runs, remove the
directories that became empty (fixing the 14-empty-dirs leak); the pinned run
stays skipped from deletion as today.

notes.mjs, decisions.mjs --live, and the idx cache writes stay pointed at the
memory store: they produce memory-shaped data the product's retention and
reindex already govern.

Lane 2 does not run the node tests; the architect runs them centrally after
relocating fixture data (copies first, destructive moves only after user
approval).

## Architect verification plan, not lane work

1. Gate: cargo fmt -- --check, cargo clippy -p qol-memory --all-targets
   --all-features --keep-going -- -D warnings, cargo test -p qol-memory
   --all-features, cargo build -p qol-memory.
2. Non-destructive fixture staging: copy the 4 non-empty snapshot dirs from
   the live store and extract the pinned notes run from
   qol-memory-notes-archive-2026-09-14.tar.gz into
   reports/qol-memory/fixtures/, then run the node test scripts
   (test-alias.mjs, test-units-replace.mjs, test-e2e.mjs,
   test-retrieval-log.mjs) and confirm they pass against the relocated
   fixtures.
3. Verify no research default writes into a scratch store: run snapshot.mjs
   and eval.mjs with a scratch QOL_MEMORY_STORE and confirm the store
   directory stays free of snapshot/, eval/, and ingest.jsonl.
4. Present the classification and the destructive step to the user. Only
   after approval: move store research artifacts to reports/, delete
   units.jsonl.bak-preprune, the 14 empty snapshot dirs, and the 5 stale
   .ingest-state.json.*.tmp files, then re-run status and doctor on the live
   store and confirm the final composition (expected about 125 MB: units
   64, notes 26, idx 31, small state files).

## Prohibitions for both lanes

Edit only the owned paths. Never run build, test, lint, format, or git
commands. Add no code comments. Use no em-dash character anywhere. No new
dependencies. Report files and lines changed plus any conscious deviations,
and nothing else.

## Blast radius, architect-verified greps

The store-audit scouts enumerated every reader of the literals this change
alters: snapshot/ pin readers (eval/eval.mjs:18-27, notes.mjs, decisions.mjs
:27-30, ingest.mjs, test-e2e.mjs:16), the Aug 12 snapshot and Aug 13 notes
pins (test-alias.mjs:12-13, test-units-replace.mjs:12-13,
eval/verdict-eval.mjs:17-19, candidates.mjs:15), store-default output paths
(snapshot.mjs:28,:62, eval/eval.mjs:24, eval/skills-eval.mjs:17,
parity.mjs:212, ingest.mjs:87-91, candidates.mjs:142, ask.mjs:32,
distinctive.mjs:69). Every hit lands in Lane 2's owned set. On the product
side the touched call sites are the warm hook (app/mod.rs:134), the rotation
callers (retrieval_log/mod.rs:124-136), and doctor registration; Lane 1 owns
them all.
