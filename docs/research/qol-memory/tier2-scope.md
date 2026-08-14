# qol-memory tier-2 consolidation: scoped build plan

Status: scoping document, grounded 2026-08-13 on live online research + measured
corpus state + the adversarial m4a1 finding. This is the plan for the next
buildable slice, not a design final.

## 1. Why tier-2 now (the measured gap)

The user's live test "how did we fix the m4a1 anchoring" returns
candidates/no-memory. The answer exists in 5 compaction summaries of session
019feb29 (kcd2-m4a1) plus 152 assistant units. The adversarial ensemble
(2026-08-13, three passes) proved:

- ask.mjs and eval.mjs hard-wire retrieval to kind==user; compaction and
  assistant units are invisible to the surface.
- Adding compaction raw as an answer layer FAILS: margin gate blocks the
  5 near-identical copies (margin ~1.0 < 2.0); score floor blocks the long
  docs (2.7k tokens vs 0.5k user under b=0.75 length normalization);
  compaction stories are point-in-time and mid-flight, so surfacing them as
  authoritative answers = silent guessing (mission-forbidden).
- The honest verdict (candidates) is correct. The real lever is write-time
  consolidation: distill the SETTLED decision out of the compaction-story
  sequence into proper notes under the notes layer.

## 2. What online research now says (verified 2026-08-13)

All primary sources fetched live (arXiv API + official docs).

### Mem0 (arXiv 2504.19413 + official docs, current OSS v3)
- Current pipeline is SINGLE-PASS ADD-ONLY extraction: input conversation ->
  retrieve top-10 related existing memories (dedupe context) -> ONE LLM call
  -> extract all distinct new facts -> batch embed -> hash-based dedupe
  (MD5) -> batch insert. No UPDATE/DELETE; memories accumulate; nothing is
  overwritten.
- This is a CORRECTION to our doc's earlier "two-phase ADD/UPDATE/DELETE"
  claim (the paper described the old v2). The field moved to ADD-only.
- Multi-signal retrieval (semantic + BM25 + entity) and temporal reasoning
  (time-aware retrieval) are platform features.
- Alignment with our architecture: append-only units + invalidate-never-delete
  is exactly the Mem0-v3 ADD-only shape. Provenance preserved.

### Anthropic context engineering (fetched live)
- Compaction: "distills the contents of a context window in a high-fidelity
  manner... preserving architectural decisions, unresolved bugs, implementation
  details while discarding redundant tool outputs". Compaction is the FIRST
  lever; structured note-taking is the SECOND. This validates pi compaction
  summaries as the write-time distillation input.
- Subagents: "explore extensively... but return only a condensed, distilled
  summary (often 1,000-2,000 tokens)". The compaction summary IS that
  subagent-shaped distillation already.
- Memory tool: file-based system to "store and consult information outside
  the context window... build up knowledge bases over time, maintain project
  state across sessions". File-based, not a DB.

### A-MEM (arXiv 2502.12110)
- Zettelkasten-style dynamic indexing + linking. When a new memory is added,
  generate a comprehensive note with structured attributes (contextual
  descriptions...). LLM-written notes at interaction time.
- Confirms: the notes are LLM-written and structured at write time, and
  linked by content, not timestamp.

### RAPTOR (arXiv 2401.18059)
- Recursive abstractive summarization into a tree. Retrieval integrates
  information at different abstraction levels.
- Relevance: compaction summaries are the natural leaf -> recursive summaries
  pattern; we are NOT building a tree now (overkill for 29 units), but the
  "summarize at write time, retrieve at multiple levels" principle holds.

### Generative Agents (arXiv 2304.03442)
- Natural-language memory + reflection (synthesize memories into higher-level
  reflections over time). The reflection step = periodic consolidation, which
  is what tier-2's "on compaction events" does.

## 3. Where we are (measured state, 2026-08-13)

- Corpus: 18,365 units (3,744 user / 14,780 assistant / 29 compaction),
  4,300 notes. Snapshot runs 2026-08-10T21-38-02-273Z (eval-pinned) and
  2026-08-11T15-39-46-033Z (live).
- Notes layer: deterministic trigger extraction (notes.mjs), 4,300 notes,
  ~4300 notes run 2026-08-11T16:25:39.517Z. Held-out 16 file facts, 12/16.
- ask.mjs: ~310ms warm, verdict answered/candidates/no-memory, --brief 2KB.
- Gates (calibrated, frozen): NO_MEMORY_COV 0.5, FLOOR 6.0, NOTE_COV 0.7,
  NOTE_SCORE 7.0, UNIT_COV 1.0, UNIT_SCORE 8.0, UNIT_MARGIN 2.0, HIGH_MARGIN
  1.8. Frozen eval: units 11/20, notes 10/10, combined 21/30, coverage 30/30.
- Staleness: RECENCY_CLS (count/status/version/flag/config) new-wins within
  a family; superseded chain surfaced. Full bi-temporal DEFERRED.
- Vision constraints: zero new deps (unless earned), compute-sacred,
  perf-first, provenance on every memory, notes = answer surface, raw
  assistant never indexed, silent degradation forbidden, first-run truth =
  "no memory of that".

## 4. The scoped build (tier-2 consolidation, write-time ADD-only)

### 4.1 What we build

A NEW deterministic+LLM hybrid "decision notes" extractor, runnable as a
workflow node, that turns the compaction-story SEQUENCE of a session into
SETTLED decision notes.

- Input: the 5-8 compaction summaries of a session (the story sequence),
  NOT raw transcripts, NOT the full session.
- Output: decision notes appended to the notes layer (a new `decision` cls
  or a `concept` cls), each with: text (the settled fact), source_key (the
  newest compaction unit key), source_ts (its ts), session, and a
  supersedes list (prior compaction members that said otherwise).
- Mechanism: the LLM (via the same harness that reads the memory, or a
  configurable model binary) runs ONE prompt per session over the compaction
  sequence: "given the sequence of compaction summaries, extract the final
  settled decisions, with the evidence chain of how they changed". The
  adversarial finding that "the story changes across compactions" becomes the
  INPUT: the LLM is explicitly asked to resolve the sequence into final state.
- Dedupe: same hash-based dedupe as notes (content-hash note key); no
  overwrite, invalidate-never-delete. New decision note supersedes older
  members via the existing superseded chain, but they are never deleted.

### 4.2 Why this fits vision (and the field)

- ADD-only accumulation = Mem0 v3 = our append-only doctrine. No UPDATE/DELETE
  machinery needed.
- Input is the compaction summary = Anthropic's "first lever" distillation
  already produced for free by pi. We are not re-distilling raw text; we are
  resolving a sequence of existing summaries into final state.
- Notes are the sanctioned answer surface (the doc's answer-layer discipline).
- Provenance: every decision note carries source_key/source_ts/session.
- Compute: ONE LLM call per session's compaction sequence, NOT per unit, NOT
  per query. 29 compaction units -> ~10-20 calls for the whole corpus once,
  then ~0-1 per session on new compactions. Bounded, batchable, background.
  This is the compute-sacred compromise: write-time distillation is the field
  consensus and it is cheap because pi already did the expensive part.

### 4.3 Explicit non-goals (what this is NOT)

- NOT raw assistant indexing (proven echo-failure, stays excluded).
- NOT surfacing compaction units as answers (adversarial refutation).
- NOT full bi-temporal as-of machinery (deferred).
- NOT embeddings / dense rerank (deferred; zero-dep BM25 is optimal now).
- NOT a watcher/daemon (tier-2 differential replay proved unfunded).
- NOT a tree (RAPTOR) — 29 units don't need recursion.
- NOT UPDATE/DELETE consolidation (Mem0 v3 says ADD-only; our doctrine
  matches).

### 4.4 The eval gate (how we know it works)

- New held-out questions from the m4a1 class: draft 3-4 questions from memory
  before reading units (draft-before-reading protocol), e.g. "what was the
  final fix for the m4a1 rifle anchoring", "did we keep the July reanchor CCD
  lane". These are the canonical compaction-only questions.
- Extend eval.mjs: a `--kinds` addition is NOT enough (compaction units are
  not notes); add a `--notes-only decision` mode or a `consolidate` step to
  the eval that runs the decision extractor then scores the decision notes.
- Acceptance: the m4a1-class questions hit at hit@1/hit@5 on the decision
  notes layer; frozen eval unchanged (notes 10/10, combined 21/30, coverage
  30/30); no new false positives on heldout 16.
- Calibration: re-run calibrate.mjs against the new decision notes to keep
  the abstention-precision curve honest (baseline 9/16 at 100%).

### 4.5 The build order (smallest slice first)

1. `decisions.mjs` — the extractor workflow node: read pinned snapshot,
   group compaction units by session, for sessions with >=2 compactions emit
   the sequence, call the LLM once per session, emit decision notes to a new
   notes run with cls "decision" + supersedes provenance. Zero changes to
   ask.mjs / eval.mjs / existing notes.
2. Draft the m4a1-class held-out questions (draft-before-reading).
3. Wire decision notes into ask.mjs's notes layer read (latest-run merge:
   decision notes join the notes pool; no new gates needed because they are
   notes-class with the existing NOTE_COV/NOTE_SCORE/MARGIN gates).
4. Run the full eval + calibrate; verify frozen numbers unchanged + m4a1
   class answered.
5. Commit to qol-memory-tier1 worktree; record iteration in the design doc.

### 4.6 Open decisions (need user call before build)

RESOLVED 2026-08-13 (user):
1. LLM mechanism: deterministic fallback (c) is the always-available
   baseline; the LLM path is configurable via env vars and defaults to
   the user's pi model. Verified live: `pi -p --provider deepseek
   --model deepseek-v4-flash --thinking low --no-session` returns a
   one-shot answer (smoke-tested OK). Env contract:
   - QOL_MEMORY_MODEL (default deepseek-v4-flash) - selectable model
   - QOL_MEMORY_PROVIDER (default deepseek)
   - QOL_MEMORY_THINKING (default low - fast inference)
   - QOL_MEMORY_MODEL_DISABLE=1 - force deterministic fallback
   decisions.mjs resolves the compaction sequence by spawning
   `pi -p --no-session` with the prompt on stdin; if the pi binary is
   missing or disabled, it degrades to the deterministic note
   (last-compaction Decision/Progress verbatim) with a visible
   source_kind="decision-deter" vs "decision". No silent degradation.
2. Note home: same notes.jsonl with a new cls "decision" (one pool,
   existing gates, no second read path). CONFIRMED (user).
3. Backfill: whole corpus once (~20 calls), because the m4a1 case is
   historical and the eval needs it. CONFIRMED (user).

## 5. Risk register (from the adversarial passes)

- Verdict regression via calibrate: a too-eager decision layer flips honest
  candidates -> wrong answered. Mitigate: decision notes must clear the same
  NOTE gates; the deterministic fallback (last-compaction verbatim) is
  conservative; re-run calibrate in the gate.
- Pollution: compaction "Next Steps" planned commands must NOT become facts.
  Mitigate: the LLM prompt explicitly says "extract only SETTLED decisions;
  never planned or pending actions; never commands"; the deterministic
  fallback only takes the Decision/Progress sections.
- Note-key churn: any extractor edit changes note keys -> heldout
  re-annotation. Mitigate: decision notes use a NEW cls; existing note keys
  untouched; heldout re-annotation is isolated to new questions.
- Cross-session echo: many sessions produce near-identical compaction
  headers. Mitigate: dedupe by content hash; the extractor strips the Goal
  header preamble from the indexed text (only Decision/Progress/Key
  sections feed the note text).
- Privacy: compaction summaries may carry unredacted paths. Mitigate:
  route decision-note text through the same redaction as snapshot units;
  decision notes are the only layer that syncs (per mission).

## 6. What success looks like

- `node ask.mjs "how did we fix the m4a1 rifle anchoring" --brief` -> answered
  (medium confidence max), with provenance (source_key = the newest compaction
  unit, session, ts) and the superseded chain showing the July-lane revert.
- Frozen eval unchanged; new held-out m4a1 questions answered.
- No new deps; compute = ~20 one-shot LLM calls + ~0/query; ~310ms ask
  unchanged.
- The notes layer (answer surface) grows a decision class without touching
  the append-only units or the verdict machinery.
