# qol-memory retrieval event log - build spec v1

Status: architect contract, DESIGN ONLY (no implementation). Grounded in
docs/research/qol-memory.md (loop architecture, design decisions, oracle
doctrine), the shipped retrieval path (ask.mjs, eval/verdict-eval.mjs,
heldout.json), the store conventions (continue-recall-scope.md,
live-capture-scope.md, seal/merge libs), and the qol-skills tool extension
(qol-memory-tool.ts, inject-qol-memory-continue.cjs).

Goal: every retrieval logs an event. Misses (no-memory verdicts, candidates
verdicts, low-confidence hits, wrong or unanswered eval results) accumulate
into candidate heldout questions. The eval gate (verdict-eval.mjs, gate PASS
invariant) remains the ONLY admission instrument for heldout growth, and the
human/architect remains the final admission. This is the closed-loop
backbone: it feeds question-set growth (replacing manual c01-c10 rounds), the
skill-intelligence loop (separately, see F), and the missed-retrieval audit
(the 8 abstentions h08/h09/p01/p02/p03/d01/d02/d03 remain future recall
work).

## Verified facts the design relies on

- Loop status: loop 1 (retrieval log) is design intent, NOT implemented;
  loop 3 exists partially, "the eval suite is operational but new questions
  are added manually with no miss-logging" (qol-memory.md:129-135); loop 3's
  compounding mechanism is "New questions come from real retrieval misses"
  (qol-memory.md:141-146). Design decision 1: "Everything observable:
  append-only event log of retrievals, hits, misses, usage" (qol-memory.md:
  163-166). Design decision 4: eval-as-artifact, question set versioned with
  the system in the repo (qol-memory.md:172).
- ask.mjs computes a verdict on EVERY invocation: answered | candidates |
  no-memory, with confidence and a reason (ask.mjs:195-291). There is no
  --verdict flag; every call is a verdict-mode call. Flags today: --store,
  --k, --brief, --exclude-session (ask.mjs:14-17, 115-116).
- The full output object (ask.mjs:333-369) carries every retrieval signal
  needed for an event: verdict, confidence, reason, gates, non_default_gates,
  answer (with key/source_ts/score/margin/superseded), recalled (top-5 note
  keys + scores), related, signals (top_note_score, top_unit_score,
  unit_margin, note_token_coverage, unit_token_coverage, max_token_coverage,
  notes_run_ts, snapshot_run_ts, live_units, stale_layer,
  recency_resolved), counts, skills. stdout is the only output
  (ask.mjs:370).
- ask.mjs already writes to the store on every call: mkdirSync + a
  manifest.json rewrite (schema 1, ask_mjs path) at ask.mjs:20-25. A store
  write on the retrieval path is established precedent.
- NO retrieval logging exists today. ask.mjs writes nothing besides the
  manifest; the tool writes only /tmp/qol-memory-tool-calls.log with the
  spawn args (qol-memory-tool.ts:133), no verdicts, not in the store. The
  deleted per-turn hook's fireLog wrote {ts, stage, ms, verdict, conf} lines
  to store/hook.log (qol-memory.md:1024-1026); the pattern survives in
  inject-qol-memory-continue.cjs:34-38 (mkdirSync + appendFileSync JSONL,
  try/catch empty, never affects exit).
- Tool spawn: node ask.mjs "<query>" --brief --exclude-session <sid>,
  spawnSync, 6000ms timeout (qol-memory-tool.ts:130-136). Response contract:
  VERDICT / FACT / PROVENANCE / SUPERSEDES / HINT / no-memory lines, plus
  the non-default-gates warning (qol-memory-tool.ts:150-164). This contract
  must not change.
- verdict-eval.mjs freezes the pinned snapshot run + notes run into
  /tmp/qol-memory-verdict-eval/<snapshot>__<notes> and runs every ask.mjs
  call against that COPY with QOL_MEMORY_STORE pointing at it
  (verdict-eval.mjs:29-44, 57-63). Eval-path ask.mjs calls therefore never
  touch the real store, including the ingest-internal informational
  verdict-eval step (ingest.mjs:59, 82).
- calibrate.mjs spawns ask.mjs against the REAL store with gate env
  overrides and no --store override (calibrate.mjs:48-51). A default-on log
  would be flooded by its synthetic gate sweeps unless the calibrate path
  opts out.
- Gate invariant, re-verified 2026-08-14 by running the harness: heldout 30
  | answered 22 | correct 22 | wrong 0 | unanswered 8 | traps 8/8 safe |
  gate PASS. The 8 unanswered are exactly h08, h09, p01, p02, p03, d01,
  d02, d03. Gate formula: wrong == 0 && correct >= FLOOR(11) && trapFails
  == 0 (verdict-eval.mjs:20, 86, 112). factMatch = normalized substring
  match, norm = lowercase + non-alnum to space + whitespace collapse
  (verdict-eval.mjs:42-47, 49-55).
- Single-note discriminator doctrine: heldout oracle facts must be verbatim
  substrings of exactly ONE note in the pinned notes run; applied when the
  degenerate d01/d03/d04 oracles were replaced (qol-memory.md:1196-1201).
- Store conventions: JSONL logs (units.jsonl, hook.log, ingest.jsonl) carry
  NO per-line schema field; schema-versioned JSON is reserved for marker
  files (manifest.json schema 1, units.seal.json, continue.marker.json
  "qol-memory-continue-v1"). Append-only files are written with
  appendFileSync; rewrites use tmp+rename (seal.js:31-35);
  units.jsonl is never rewritten by readers.
- Kill-switch convention: QOL_MEMORY_LIVE_CAPTURE_DISABLE
  (qol-memory-tool.ts:16), QOL_MEMORY_CONTINUE_DISABLE
  (inject-qol-memory-continue.cjs:38), QOL_MEMORY_HOOK_DISABLE (deleted
  hook), QOL_MEMORY_DISTILL. Env-driven, checked before any work.
- ask.mjs warm latency ~0.35s: warm 0.341-0.355s, warm-after-stale
  0.350-0.366s (qol-memory.md:1235-1236). The log write must add ~1ms, not
  milliseconds more.
- Privacy precedent: "an opt-in query log with retention" was a deferred
  privacy item since the thirteenth pass (qol-memory.md:681, 714, 729). This
  log is that item, with the kill-switch as the opt-out.
- skills-eval.mjs is a SEPARATE retrieval surface (BM25 over skill metadata
  docs + anchor check + glossary flags, skills-eval.mjs:31-64, 100-112);
  its misses are description-vocabulary gaps that earn standards-evolution
  description edits, then the alias prunes (qol-memory.md:993-995,
  1011-1013).
- The task context references field research at /tmp/memory-research.md
  (held-out-gated self-improvement, Regimes, log everything). That file no
  longer exists on disk (verified 2026-08-14); the doctrine is grounded
  here in qol-memory.md:129-146 and 163-166 instead.

## Architecture

```
any retrieval (three sources)
  ├─ qol_memory_retrieve tool  -> node ask.mjs --log-source tool --log-cwd <cwd>
  ├─ ask-cli (manual)          -> node ask.mjs                        (source default)
  └─ verdict-eval.mjs          -> node ask.mjs --log-source eval [--log-fact <fact>]
                                    (runs against the /tmp frozen store, never the real one)

ask.mjs (single choke point)
  └─ computes verdict + signals (existing, untouched logic)
       └─ lib/retrieval-log.js appendRetrieval(out)  (NEW, after verdict, before stdout)
            ├─ statSync size check, append one JSONL line to <store>/retrievals.jsonl
            ├─ tail-cap rotation at 10MB (rare, newline-boundary, double-checked)
            └─ kill-switch QOL_MEMORY_RETRIEVAL_LOG_DISABLE=1, --no-log flag
                 (calibrate.mjs uses --no-log: synthetic sweeps are not retrievals)

harvest (on-demand, human-invoked)
  └─ node candidates.mjs --store <store>
       ├─ scans retrievals.jsonl for misses (source ask-cli|tool,
       │    verdict no-memory|candidates), dedupes by norm_query vs
       │    heldout.json + candidates.jsonl, 24h cooldown per norm_query
       └─ appends candidates to <store>/candidates.jsonl (status candidate)
            + writes report.json to <store>/ingest/ for architect review

promote (human-invoked, gate-gated, the ONLY admission path)
  └─ node candidates.mjs --promote <key>
       ├─ runs the verdict-eval gate with heldout.json + the candidate
       │    (wrong==0, correct>=FLOOR, traps safe) on the pinned frozen store
       ├─ verifies the single-note discriminator (fact is a verbatim
       │    substring of exactly one note in the pinned notes run)
       └─ only on PASS: appends the question to eval/heldout.json (repo),
            flips candidates.jsonl status to promoted

visibility (read-only, no behavior change)
  ├─ ingest.mjs report gains candidates_pending count (ingest.mjs:62-89)
  └─ verdict-eval.mjs output gains "candidates pending N" (informational)

The retrieval path NEVER reads retrievals.jsonl or candidates.jsonl.
The gate is the acceptance instrument; the human/architect is the final
admission. No LLM anywhere on the log, harvest, or promote path.
```

## Design decisions

### A. Event schema

Recommendation: one JSONL line per retrieval (one per ask.mjs invocation),
fields:

```json
{
  "ts": "2026-08-14T08:00:00.000Z",
  "source": "tool",
  "session": "019f…",
  "cwd": "/media/kmrh47/WD_SN850X/Git/worktrees/…",
  "query": "how did we fix the m4a1 anchoring",
  "verdict": "candidates",
  "confidence": "low",
  "correctness": null,
  "latency_ms": 356,
  "k": 5,
  "exclusion": { "exclude_session": true, "non_default_gates": false },
  "gates": { "NO_MEMORY_COV": 0.5, "FLOOR": 6, "NOTE_COV": 0.5, "NOTE_SCORE": 6, "UNIT_COV": 1, "UNIT_SCORE": 8, "UNIT_MARGIN": 1.5, "HIGH_MARGIN": 1.8 },
  "signals": { "top_note_score": 9.3, "top_unit_score": null, "unit_margin": null, "note_token_coverage": 0.53, "unit_token_coverage": 0.4, "max_token_coverage": 0.53, "notes_run_ts": "2026-08-13T16:31:40.844Z", "snapshot_run_ts": "live", "live_units": true, "stale_layer": false, "recency_resolved": false },
  "answer_key": null,
  "recalled_keys": ["53b347117db02b9d", "…"],
  "counts": { "units": 4376, "notes": 3522 }
}
```

Rationale:

- One line per verdict-mode call, written by ask.mjs itself (see B), is the
  full observability point: the tool, the CLI, and eval all route through
  the same choke point, and only ask.mjs holds the signals (margins,
  coverage, gate values, recalled keys) at output time.
- `source` is explicit, never inferred: ask.mjs gains --log-source
  (ask-cli|tool|eval, default ask-cli); the tool passes --log-source tool,
  verdict-eval.mjs passes --log-source eval. Heuristics (brief flag,
  exclude-session presence, store path) are fragile and rejected.
- `correctness` is null at write time for tool/CLI sources: correctness is
  unknowable from the retrieval alone. It is populated only by eval-source
  events, where verdict-eval.mjs passes --log-fact <q.fact> and ask.mjs
  applies the same normalized substring match as factMatch
  (verdict-eval.mjs:49-55): correct (answered + match), wrong (answered
  without match), unanswered (not answered), and for trap questions
  untrapped (not answered, safe) or trapped (answered, the gate-fail case
  that must never occur). The norm() used for this match is the SAME
  lowercase/alnum/collapse function verdict-eval uses, so eval annotation
  and gate scoring cannot disagree.
- `session` = the --exclude-session value (the querying session for tool
  calls), null for pure CLI. `cwd` = --log-cwd when the caller provides it
  (the tool has ctx.sessionManager.getCwd()), else null. Both nullable by
  design.
- `gates` + `signals` are copied verbatim from the existing out object
  (ask.mjs:333-369), so the event is a faithful projection of the retrieval
  with zero re-derivation.
- `recalled_keys` = the top-5 note keys from out.recalled. It is the raw
  material for candidate formation (C) and the future round-trip grounding
  check (open questions).
- Schema versioning: NO per-line schema field, mirroring the store's JSONL
  convention (units.jsonl, hook.log, ingest.jsonl carry none); the
  schema-versioned convention is reserved for marker files. The version
  lives as a pointer in manifest.json, which ask.mjs already rewrites on
  every call (ask.mjs:20-25): "retrievals": "qol-memory-retrieval-v1" and
  "candidates": "qol-memory-candidates-v1". Readers drop unparseable lines
  (the parseUnitsText precedent, seal.js), which also absorbs the
  mid-append partial-line case.

### B. Log location + write path

Recommendation: <store>/retrievals.jsonl, append-only, written by ask.mjs
ITSELF on every verdict-mode call, after the verdict is computed and before
stdout. No atomic rename needed (append-only); a simple tail cap handles
size. The tool wrapper does NOT write events.

Rationale:

- ask.mjs is the single retrieval choke point: tool calls, manual CLI asks,
  and eval runs all pass through it. Logging in the wrapper would lose CLI
  asks (a first-class retrieval source for the closed loop) and would lack
  the signals (margin, coverage, gate values) that make a miss analyzable.
- ask.mjs already writes to the store on every call (the manifest rewrite,
  ask.mjs:20-25), so a store write on this path is established behavior,
  not a new responsibility.
- Crash safety: appendFileSync issues one write() with O_APPEND per line; a
  single write() to a regular file is atomic on Linux for line-sized
  payloads, so concurrent appends (two sessions calling the tool at once)
  cannot interleave or tear. The reader-side partial-line guard is free
  insurance anyway (parseUnitsText drops a trailing unparseable line).
- No tmp+rename: the seal.js:31-35 pattern protects files that readers
  read while a writer rewrites them. retrievals.jsonl is never read by the
  retrieval path, only appended, so rename adds complexity with no benefit.
- Size control: a tail cap, not sealing. When the file exceeds CAP (10MB
  default), the append path rewrites the last TAIL (1MB) slice cut at a
  newline boundary. The rotation is double-checked (re-stat before
  truncate) and can occur at most once per cap crossing. Estimate: ~1.5KB
  per event at 10-40 real retrievals/day = 15-60KB/day, so 10MB is roughly
  6-18 months. Sealing (units.seal.json/.gz) exists because the corpus is
  multi-MB and index reads traverse it; a capped log needs none of it, and
  hook.log today is uncapped at 7.6KB without issue. The cap is cheap
  insurance, and the log is the deferred "opt-in query log with retention"
  privacy item (qol-memory.md:681, 714, 729): bounded retention is part of
  that contract.
- Kill-switch: QOL_MEMORY_RETRIEVAL_LOG_DISABLE=1 (the established env
  convention) plus a --no-log flag for harness spawns. calibrate.mjs passes
  --no-log (its gate sweeps are synthetic probes against the real store,
  not retrievals; calibrate.mjs:48-51). test-e2e and the verdict gate run
  against /tmp sandbox or frozen stores, so their events land there by
  store resolution alone.

### C. Miss semantics

Recommendation: a miss is an event whose verdict is no-memory or candidates
(knowable at log time), OR whose annotated correctness is wrong, unanswered,
or trapped (knowable only through eval annotation). Low-margin answered
(margin < HIGH_MARGIN = 1.8x) is a monitored soft hit, logged but NOT
harvested in v1. Harvest (candidate formation) uses only source
ask-cli|tool events; eval-source misses are excluded because they ARE the
heldout suite already.

Rationale:

- The verdict classes map exactly to the audit: the 8 abstentions
  (h08/h09/p01/p02/p03/d01/d02/d03) all produced no-memory or candidates
  verdicts on the 2026-08-14 gate run. no-memory = "first-run truth = no
  memory of that, never silent guessing" (qol-memory.md:696);
  candidates = "no decisive answer: note_cov=… unit_cov=…" (ask.mjs reason
  string). Both are honest abstentions and both are the raw material for
  question growth (loop 3: "New questions come from real retrieval
  misses", qol-memory.md:141-146).
- Low-margin answered is deliberately NOT a v1 candidate source. The gates
  are calibrated (baseline gates answer at 100% precision on the frozen
  eval; today 22/30 correct, 0 wrong), so an answered verdict is the
  system's honest operating point; harvesting every medium-confidence
  answer would flood candidates with questions the system already answers.
  The event still logs the margin, so the class is monitorable.
- Wrong/unanswered/trapped are only knowable after scoring against a gold
  fact, which only the eval gate does. For eval-source events the
  correctness annotation rides on the event itself (A). For tool/CLI
  events there is no post-hoc channel in v1; the round-trip grounding
  check (re-retrieve with the answer as query) is the documented future
  annotation path (open questions). The eval gate remains the ONLY
  instrument that ever produces a correctness verdict.
- Miss to candidate: a harvested event becomes a candidate carrying the
  unique-fact discriminator payload: query verbatim, norm_query, the
  verbatim text of the top surfaced evidence (the answer note text if
  answered, else recalled[0]'s note text), the normalized form of that
  text, and the source unit/note key. The doctrine (qol-memory.md:
  1196-1201) requires every heldout fact to be a verbatim substring of
  exactly one note in the pinned notes run; auto-growth therefore carries
  the source key + scorer-normalized verbatim text so the promote step can
  verify discrimination without re-derivation, and the human can refine
  the fact slice at promote time.
- Dedupe: one pending candidate per norm_query (norm_query = the
  verdict-eval norm: lowercase, non-alnum to space, whitespace collapse,
  verdict-eval.mjs:42-47), checked against heldout.json questions AND
  existing candidates.jsonl entries. Minimum ts gap: 24h per norm_query, so
  a burst of near-identical misses (a session asking the same question five
  times) yields one candidate; a miss a week later can re-capture.
  Deterministic first-wins ordering by event ts.

### D. Candidate store

Recommendation: <store>/candidates.jsonl, a separate file, NOT a section of
the event log. One JSON line per candidate:

```json
{
  "key": "sha256(norm_query) hex 16",
  "query": "how did we fix the m4a1 anchoring",
  "norm_query": "how did we fix the m4a1 anchoring",
  "fact": "full-body 243 controllers",
  "fact_norm": "full body 243 controllers",
  "source_unit_key": "79046028d14b1cec",
  "source_event_ts": "2026-08-14T08:00:00.000Z",
  "source": "tool",
  "session": "019f…",
  "cwd": null,
  "verdict": "candidates",
  "created_ts": "2026-08-14T09:00:00.000Z",
  "status": "candidate",
  "promoted_ts": null,
  "heldout_id": null,
  "rejected_ts": null,
  "reject_reason": null
}
```

Rationale:

- The event log is append-only and write-only; candidates have a lifecycle
  (candidate -> promoted | rejected) that requires read-modify-write. Mixing
  the two consistency models in one file would force rewrites of the
  append-only log, violating its contract. Separate file, same store root,
  same JSONL convention, version pointer in manifest.json.
- key = sha256(norm_query) slice(0,16), the same key family as unitKey
  (sha256 hex slice, qol-memory-tool.ts:48-51): stable across re-harvests
  and idempotent by construction.
- Lifecycle: status candidate until a human/architect runs the promote
  command or an explicit --reject <key> --reason. There is NO automatic
  promotion and NO automatic rejection; stale candidates are a visibility
  item, not a policy.
- Promotion rule (the gate stays the admission instrument): the promote
  command (1) builds a temporary heldout = committed eval/heldout.json +
  the candidate question, (2) runs the existing verdict-eval gate on the
  pinned frozen store with that file (wrong == 0, correct >= FLOOR, traps
  8/8 safe; verdict-eval.mjs:86), (3) verifies the single-note
  discriminator (the candidate fact must be a verbatim substring of exactly
  one note in the pinned notes run; qol-memory.md:1196-1201), (4) only on
  PASS appends the question to eval/heldout.json (the repo artifact,
  eval-as-artifact, qol-memory.md:172) and flips the candidate to promoted.
  A candidate that fails the gate or the discriminator check never
  promotes; the command exits non-zero with the gate output as the reason.
- Gate-local discrimination: the new question must not regress any existing
  invariant (wrong==0, traps safe, floor met) AND must itself be answered
  correctly by the current system; a question the system cannot answer
  correctly FAILS the gate with itself included and is refused. This is the
  "changes gated by a baseline that only grows" mechanism
  (qol-memory.md:144-145) applied to question growth.

### E. Cadence

Recommendation: harvest on demand, `node candidates.mjs --store <store>`
(the seal.mjs manual-step precedent), NOT on ingest and NOT on eval.
Surfacing: candidates.mjs writes report.json to <store>/ingest/ (the
workflow-node report.json precedent) listing every proposal with its event
provenance for architect review; the ingest report gains a read-only
candidates_pending count; verdict-eval's output line gains a read-only
"candidates pending N" (informational, exit code untouched).

Rationale:

- ingest.mjs is a backfill pipeline (snapshot -> merge -> distill -> evals,
  21.5s, deterministic report; ingest.mjs:62-89). Auto-harvesting inside it
  would make ingest's output depend on the retrieval log's history and add
  a write path to a pipeline whose contract is determinism.
- The eval is the gate, not the collector: it must stay byte-identical
  across runs (the frozen-eval invariant), and a harvest inside it would
  couple the two.
- Harvesting is a judgment-light but not judgment-free step (fact slice
  choice, dedupe edge cases), so the human-invoked command + report.json
  review loop is the honest shape, exactly like seal.mjs and the promote
  step. The count surfaces (ingest report, verdict-eval line) keep pending
  candidates visible without coupling.

### F. Skill-intelligence loop

Recommendation: stay separate. The event log does NOT subsume
skills-eval.mjs's miss list.

Rationale:

- Different surface: skills-eval.mjs scores BM25 over skill METADATA docs
  (buildMetaDoc + anchor check + glossary flags, skills-eval.mjs:31-64,
  100-112); ask.mjs retrieves transcript memory. The event log covers the
  latter only.
- Different miss meaning and fix path: a skills miss is a
  description-vocabulary gap that earns a standards-evolution description
  edit, then the glossary alias prunes (qol-memory.md:993-995, 1011-1013);
  a transcript miss is a candidate heldout question admitted by the gate.
  The two loops converge only in the principle (every miss list is visible,
  both appear in the ingest report) and must not share a store: a skills
  miss has no note key to carry as a discriminator.
- v1 adds no skills fields to the event; if the skill loop later wants
  retrieval-side observability, ask.mjs's skills output (out.skills,
  ask.mjs:333-369) is already loggable as a projection.

### G. Cost + determinism

Recommendation: one statSync size check + one appendFileSync after the
verdict is computed: ~0-1ms worst case on a ~0.35s warm retrieval
(qol-memory.md:1235-1236). No LLM, no index read or build, no log read on
the retrieval path. The write can never influence retrieval results or the
tool's response contract.

Rationale:

- The append is the same syscall family the live-capture handlers measured
  at 0-1ms (live-capture-scope.md verified-facts), and ask.mjs already
  performs an equivalent store write (the manifest rewrite) on every call
  without any measured cost.
- The tool's 6s spawnSync timeout has ~5.6s of headroom after the ~0.35s
  retrieval; 1ms changes nothing.
- Read-side neutrality is by construction: ask.mjs never reads
  retrievals.jsonl or candidates.jsonl; the append happens after the
  verdict chain (ask.mjs:195-291) and the out object (ask.mjs:333-369) are
  fully computed; the write is wrapped in the fireLog try/catch-empty
  pattern (inject-qol-memory-continue.cjs:34-38) and cannot affect stdout,
  exit code, or gate pass/fail. The frozen-eval invariant (22/22/0/8,
  traps 8/8, PASS) is byte-identical with the log present (test H10).
- Determinism: the event is a pure function of the retrieval result plus
  the wall clock; no sampling, no randomness, no LLM.

### H. Test plan

test-retrieval-log.mjs, the established sandbox pattern (tmpdir store via
QOL_MEMORY_STORE, check() pass/fail lines, non-zero exit on failure;
test-seal.mjs:16-37, test-cadence.mjs). Pure logic (append, normalize,
dedupe, rotate, discriminate) lives in lib/retrieval-log.js so tests run
without spawning; ask.mjs-level tests spawn against seeded sandbox stores.

1. Append per call: 3 ask.mjs verdict-mode runs on a seeded sandbox store
   (units + a notes run) produce exactly 3 retrievals.jsonl lines with all
   schema fields (ts, source, session, cwd, query, verdict, confidence,
   correctness null, latency_ms, gates, signals, recalled_keys, counts).
2. Kill-switch: QOL_MEMORY_RETRIEVAL_LOG_DISABLE=1 -> zero appends.
3. --no-log: spawn with the flag -> zero appends (the calibrate path).
4. Source propagation: --log-source tool / --log-source eval land in the
   event; default is ask-cli.
5. Eval annotation: spawn with --log-fact; answered+match -> correct,
   answered+no-match -> wrong, abstained -> unanswered, trap not answered
   -> untrapped, trap answered -> trapped.
6. Miss detection: harvest over a log with 2 miss events and 1 answered
   event yields exactly the 2 candidates.
7. Dedupe: second event with the same norm_query -> skipped; a query
   matching a heldout.json norm query -> skipped; a re-miss within 24h ->
   skipped; after the cooldown -> captured.
8. Rotation: synthetic oversized log -> truncation lands on a newline
   boundary, the file is valid JSONL, no partial line survives.
9. Promotion gate: a candidate whose fact is not a single-note discriminator
   (or whose inclusion fails the gate) never promotes and exits non-zero; a
   candidate that passes the gate + discriminator check promotes: heldout
   file gains the question, candidates.jsonl status flips to promoted with
   promoted_ts + heldout_id.
10. Read-side neutrality: ask.mjs stdout is byte-identical with and without
    a populated retrievals.jsonl (including a >cap file); the full
    verdict-eval gate run is byte-identical to the frozen invariant
    (22/22/0/8, traps 8/8, PASS).
11. Determinism: two harvest runs on the same log produce identical
    candidates.jsonl; two promote evaluations produce identical gate
    output.
12. Concurrent appends: two parallel ask.mjs spawns both land intact lines
    (O_APPEND single-write atomicity).

Tests never touch the real store (~/.local/share/qol-tray/plugins/
qol-memory/); all sandboxes live in tmpdir, mirroring test-seal.mjs and
test-cadence.mjs.

### I. Non-goals for v1

- No auto-promotion into heldout without the gate: the gate is the
  acceptance instrument and the human/architect remains the final
  admission. The promote command is the only path into eval/heldout.json.
- No LLM anywhere: not on the log path, not on harvest, not on promote.
- No scoring of candidates: candidates are questions with gold facts, not
  scored entries; the gate scores them at promote time.
- No per-prompt retrieval: the deleted per-turn hook stays deleted
  (qol-memory.md:1053-1056); the log observes the on-demand surface only.
- No changes to the retrieve tool's response contract: the spawn args gain
  flags, the VERDICT/FACT/PROVENANCE/HINT output does not change
  (qol-memory-tool.ts:150-164).
- No sealing of the retrieval log: the tail cap is the retention mechanism.
- No sync of the log: local-only like continue.marker.json; profile-sync
  carries distilled notes, never query logs (privacy boundary,
  qol-memory.md:681).
- No post-hoc correctness channel for tool/CLI events in v1 (the
  round-trip grounding check is future work, open questions).
- No skills fields in the event, no changes to skills-eval.mjs or the
  glossary mechanism (F).

## Cost budget

- Retrieval path: +1 statSync +1 appendFileSync ~0-1ms on a ~0.35s warm
  ask.mjs (qol-memory.md:1235-1236). No LLM, no index work, no reads of the
  log. Tool timeout headroom: ~5.6s unchanged in spirit (6000ms timeout,
  qol-memory-tool.ts:134).
- Store: ~1.5KB per event; 10-40 real retrievals/day ~ 15-60KB/day; 10MB
  cap ~ 6-18 months before the first rotation. Rotation is one rewrite,
  newline-boundary, double-checked.
- Eval: verdict-eval events land in the /tmp frozen store (verdict-eval.mjs:
  29-44), never the real store; the gate output line gains one informational
  count.
- Harvest/promote: on-demand only, seconds, deterministic, report.json
  artifacts.

## Gates

- The eval gate (verdict-eval.mjs:86) is unchanged: wrong == 0, correct >=
  FLOOR(11), traps 8/8 safe, exit 0 = PASS. The log adds observability, not
  a second gate.
- The promotion gate is the SAME gate evaluated with the candidate included
  (D): a question the system cannot answer correctly fails the gate with
  itself in the suite and never enters heldout.json.
- The frozen-eval invariant must stay byte-identical after this work:
  heldout 30 | answered 22 | correct 22 | wrong 0 | unanswered 8 | traps
  8/8 safe | gate PASS (re-verified 2026-08-14), eval units 8/30 11/30 mrr
  0.329, skills pass, test-index-incremental ALL PASS, test-seal ALL PASS,
  test-cadence ALL PASS, test-e2e ALL PASS.

## Integration points

- ask.mjs:19-25 (STORE_ROOT + the manifest write precedent; manifest gains
  the "retrievals" and "candidates" version pointers), ask.mjs:195-291
  (verdict chain, untouched), ask.mjs:333-369 (out object, the event
  projection source), ask.mjs:370 (insert appendRetrieval before
  console.log). New: lib/retrieval-log.js (append, normalizeQuery, miss
  predicates, rotation, discriminator count).
- qol-memory-tool.ts:130-136 (spawn args gain --log-source tool and
  --log-cwd <ctx.sessionManager.getCwd()>; response contract at
  qol-memory-tool.ts:150-164 untouched).
- verdict-eval.mjs:57-63 (runAsk gains --log-source eval and --log-fact
  <q.fact>; events land in the /tmp frozen store, verdict-eval.mjs:29-44);
  verdict-eval.mjs:110 (output line gains "candidates pending N",
  informational).
- calibrate.mjs:48-51 (runAsk gains --no-log; synthetic sweeps are not
  retrievals).
- candidates.mjs (NEW): harvest, --promote <key>, --reject <key> --reason;
  report.json to <store>/ingest/.
- ingest.mjs:62-89 (report gains a read-only candidates_pending count).
- eval/heldout.json: grows ONLY through the promote command, committed to
  the repo (eval-as-artifact, qol-memory.md:172).
- Manifest.json (written at ask.mjs:20-25): "retrievals":
  "qol-memory-retrieval-v1", "candidates": "qol-memory-candidates-v1".

## Non-goals (summary)

- No auto-promotion; the gate + human/architect are the admission.
- No LLM anywhere; no scoring of candidates; no per-prompt retrieval.
- No changes to the tool response contract; no sealing of the log; no sync
  of the log; no skills fields; no post-hoc correctness channel in v1.

## Open questions

- FLOOR scaling: the gate floor is an absolute count (11, verdict-eval.mjs:
  20). As promotion grows the suite past 30, the same absolute floor lowers
  the recall bar (11/40 vs 11/30). Recommendation: keep absolute until
  heldout reaches ~40, then revisit (e.g. ceil(0.5 * N), which at 30 would
  be 15 <= the observed 22); wrong == 0 and traps-safe remain the hard pins
  either way. This is a gate-semantics change and belongs in a separate
  decision.
- Cooldown bounds: the 24h per-norm_query cooldown and the 10MB cap are
  design guesses, not field-calibrated (the same status as continue-recall
  MIN_DELTA/k). The first weeks of real log data should confirm firing
  volume and candidate quality before any tuning.
- Correctness for tool/CLI events: the round-trip grounding check
  (re-retrieve with the answer as query, downgrade on failure) was
  field-grounded as a zero-dep calibration lever (qol-memory.md:836-838)
  and is the natural future annotator for correctness on non-eval events.
  v1 leaves correctness null for those sources; the recalled_keys field is
  the raw material.
- Log cwd for CLI callers: --log-cwd is populated by the tool only; a
  manual CLI ask that passes --exclude-session without --log-cwd loses the
  cwd. Deriving cwd from the excluded session's own units (units carry
  cwd) is an alternative; explicit flag wins for v1 simplicity, cwd stays
  nullable.
- Field-research provenance: /tmp/memory-research.md (Regimes,
  held-out-gated self-improvement) no longer exists on disk; this doc
  grounds the same doctrine in qol-memory.md:129-146 and 163-166. If the
  architect has that file elsewhere, the Regimes reference can be
  re-attached.
