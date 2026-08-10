# qol-memory: recursive agent memory (research + design)

Status: living document. Iteratively distilled from research and design conversations.
Each revision appends to the Iterations section; nothing is deleted, only refined.

## 1. Vision

A memory system for qol that converges toward a recursively self-improving
architecture. Not self-modifying code. Closed feedback loops where each cycle's
output raises the next cycle's starting point.

The product is the loop, not the index.

The goal is to make space for the potential: every tier of sophistication
(keyword, embeddings, learned compression, consolidation) plugs into the same
architecture without redesign.

## 2. What the field does (verified sources)

Common thread across all serious agent-memory systems: distill at write time,
retrieve by tool on demand, consolidate against bloat. SOTA is not big RAG.

- Mem0 (arxiv 2504.19413): two-phase pipeline, extraction then memory update
  with explicit ADD / UPDATE / DELETE / NONE operations. Memory is a managed
  store with write-time LLM consolidation, not raw transcript chunks.
- MemGPT / Letta (arxiv 2310.08560): OS-paging metaphor. Core memory in
  context, archival memory retrieved, recursive summarization.
- Zep / Graphiti (arxiv 2501.13956): temporal knowledge graph over
  conversations. Enterprise-grade, heavy.
- A-MEM (arxiv 2502.12110): Zettelkasten-style linked notes, LLM-written at
  interaction time.
- LOCOMO (arxiv 2402.17753): the honest warning. Long-term memory evals barely
  exist and none transfer to code-agent transcripts.
- Anthropic "Effective context engineering for AI agents" (Sept 2025):
  structured note-taking (agentic memory), retrieval by tool, subagents
  distilling 10k+ token explorations into 1-2k token summaries.
- Claude Code memory (code.claude.com/docs/en/memory): CLAUDE.md for behavior,
  auto-memory notes for accumulated learnings, both plain files.
- ICAE (arxiv 2307.06945): LoRA-trained context compression into fixed gist
  tokens, ~1% params, 4x compression, quality holds regardless of conversation
  length. The "low-rank during context retrieval" idea. Independent audit
  (arxiv 2412.17483) shows gist-token gains are task-dependent. Rejected as a
  component (trained model, prefix injection breaks caching, lossy on exact
  names) but its shape is instructive: fixed-slot recall is achieved by
  distilling at write time into a small fixed store, which is what compaction
  summaries already do without learned weights.

Pi already produces the distillation layer for free: compaction summaries
(Goal / Progress / Decisions / Next Steps), branch summaries, and file-op
lists, all structured in the session JSONL format.

## 3. Corpus facts (measured 2026-08-10)

- pi sessions: 72 files, 165 MB under ~/.pi/agent/sessions (live, grows)
- Claude Code: 749 files, 1.2 GB under ~/.claude/projects
- Raw layer is huge. Distillable layer (user messages + summaries + file ops)
  is small. Embeddings, if used, cover the distilled layer only.

Snapshot node results (docs/research/qol-memory/snapshot.mjs, schema v2,
run 2026-08-10T17-35-26-351Z):

- 5481 distilled units: 5458 user, 23 compaction, 0 branch
- Chars: user 17.3M, assistant 6.3M, thinking 30.2M, tool 28.2M, summaries 446k
- Thinking is the largest text layer and is deliberately not indexed
- Earlier ad-hoc sample numbers (user 0.31M / assistant 16.4M) were a
  counting artifact: the sample script serialized raw content JSON, which
  includes thinking blocks, and it never counted claude-type entries. The
  snapshot node numbers above are canonical.
- Claude Code corpus contains no summary / compaction entries at all; the
  distilled compaction layer comes from pi only and is sparse (23 units)
- Language mix: stratified sample across both sources (pi 62 / claude 438
  of 500), 0 / 500 non-Latin (0.0%). English-only corpus; multilingual
  models (bge-m3, qwen3) are unneeded

## 4. Retrieval engineering facts

Verified small embedding models (HF cards, MTEB avg):

| Model | Params | Dims | Seq | MTEB | Notes |
|---|---|---|---|---|---|
| all-MiniLM-L6-v2 | 22.7M | 384 | 256 | ~56 (sbert docs) | smallest, weakest |
| bge-small-en-v1.5 | 33.4M | 384 | 512 | 62.17 | default candidate |
| bge-base-en-v1.5 | 109.5M | 768 | 512 | 63.55 | +1.4 pts, 3x params |
| nomic-embed-text-v1.5 | 136.7M | 768 matryoshka | 8192 | 62.28 | dim truncation 64-768 |
| Qwen3-Embedding-0.6B | 595.8M | 1024 | 32k | multilingual | 8B sibling #1 MTEB-multi |

- Small English models cluster at 62-63.5 MTEB. Each extra point costs 3-4x
  params. On agent transcripts the gap will be smaller.
- int8 (4x smaller) and binary (32x) variants ship in FlagEmbedding. Exact
  retention on our corpus is a micro-benchmark item, not a citation.
- Hybrid BM25 + dense with RRF beats either alone (BEIR, arxiv 2104.08663,
  documents dense OOD weakness). Agent transcripts are keyword-dense: file
  paths, commands, identifiers. BM25 stays strong there.

## 5. Rust-native dependency matrix

| Option | Runtime deps | Status |
|---|---|---|
| candle-core 0.11 | none | already in monorepo (qol-voice) |
| tokenizers 0.23 | none | pure Rust |
| tantivy 0.26 | none | pure Rust BM25/full-text |
| fastembed 5.17 | ONNX Runtime (C) | rejected on dep policy |
| sqlite-vec 0.1.9 | sqlite (C) | only if multi-writer is real |
| redb 4.1 | none | pure Rust KV |
| Ollama | daemon | rejected. candle makes inference a library call |

## 6. The architecture

### Tiers

- Tier 0 (zero new deps): tantivy full-text over raw sessions + structured
  extraction of summaries / user messages / file ops + recency via qol-frecency.
  Genuinely competitive because the corpus is keyword-dense.
- Tier 1 (candle): bge-small-en-v1.5 or nomic v1.5 truncated, embeddings over
  the distilled layer only, flat in-memory cosine scan (personal scale, no
  ANN), RRF-fused with Tier 0 lexical.
- Tier 2 (consolidation): LLM writes ADD / UPDATE / DELETE memory notes on
  compaction events, reusing pi's summary output, with age-out. The SOTA
  mechanism, cheapest to add because distillation already happened.

### The loops (recursion)

1. Retrieval loop: log every query, hits, and post-retrieval behavior
   (re-read after retrieval = miss signal). Feed misses into distillation.
2. Distillation loop: retrieved-often = keep or refresh, never-retrieved =
   consolidate or delete. Usage stats drive Mem0-style operations.
3. Eval loop: held-out question set becomes a permanent regression suite in
   the repo. Every system change runs it. New questions come from real
   retrieval misses. The compounding mechanism and the honest answer to
   "recursive improvement": changes gated by a baseline that only grows.
4. Skill loop: memory surfaces recurring patterns, standards-evolution encodes
   them as skills, future sessions start from the new baseline. The system's
   output improves the system's own instructions.

### Existing qol machinery to build on, not rebuild

- qol-trace: runtime probes to logs to analysis to fixes
- standards-evolution: practice to skill to better next session
- qol-sessions: architect to implementation agent to review to accept/reject
- qol-workflow-nodes: scriptified workflows with report.json
- qol-watch: fs-change primitives (the indexer's watcher)
- qol-frecency: ranking primitive
- qol-terminal-sessions: parallel-chats awareness (cli-sessions)
- qol-profile-sync: memory DB can ride profile export (portability)

### Design decisions that make space for the potential

1. Everything observable: append-only event log of retrievals, hits, misses,
   usage. Recursion needs data about its own behavior. Cheap now, impossible
   to retrofit.
2. Index is dumb, distillation policy is upgradeable: storage schema must not
   assume keyword, embeddings, or learned compression. Policy layer is where
   ICAE-style learned compression can plug in later.
3. Provenance on every memory: source session, entry id, timestamp. Nothing
   can be revised or consolidated without traceability. Also what makes eval
   regression tests possible.
4. Eval-as-artifact: question set versioned with the system, in the repo.
5. Feedback hooks on day one: memory reads and writes emit trace probes; a
   periodic consolidation job consumes usage stats.

### Cache and context interplay (hard rule)

Prompt caching is prefix-based. Retrieval via a tool call preserves the
prefix. Injected memory restructures it and invalidates the cache every turn.
Never inject memory into the prompt. Anthropic's guidance matches: minimal
stable system prompt, everything else behind tools.

## 7. MVP seed: the first loop

The index is not the MVP. Observability + eval is the MVP.

- Append-only retrieval event log (query, hits, usage, post-retrieval behavior)
- 30-question held-out suite from real corpus history, hit@1 / hit@5
- Corpus snapshot script (workflow node, report.json, schema v2, stable unit
  keys, --keep retention, stratified language check)
- Language mix check (decides English-only vs multilingual)

Eval-harness acceptance criteria (from the review board, reqs-1): 30
questions checked in under docs/research/qol-memory/eval/ with per-question
provenance (snapshot run id + unit key + category: file/command fact |
decision | context); one command runs the harness against the pinned
snapshot run and emits report.json; hit@1 / hit@5 per strategy; zero-dep
BM25 baseline first, dense/hybrid deferred to the tier increments;
deterministic; baseline committed; new questions sourced from real
retrieval misses.

Question-drafting protocol (arch-5): draft questions from memory of the
work, before reading snapshot units. Never copy units verbatim into repo
artifacts (security-2).

Every later tier plugs into this without redesign.

## 8. Open research questions

1. MTEB does not transfer to code-agent transcripts (LOCOMO lesson). The
   held-out suite on our own corpus is the real eval.
2. int8 / binary / matryoshka retention measured on our corpus.
3. How much assistant text earns indexing: summaries-only vs full FTS.
4. Language mix in the corpus. ANSWERED: 0% non-Latin over 500 sampled user
   units, English-only. bge-small-en-v1.5 suffices; multilingual models
   dropped from consideration.
5. Whether compaction summaries alone answer recall questions. They are
   decision-dense but lose specific file / command facts, which is what the
   file-op details field covers.
6. Multi-writer reality: daemon single-writer likely kills sqlite-vec; redb
   or plain files suffice.
7. Learned compression (ICAE-style) as a future Tier 3 policy, plugged into
   the policy layer, not the storage.

## 9. Sources

- HF cards: bge-small/base-en-v1.5, nomic-embed-text-v1.5, Qwen3-Embedding-0.6B,
  bge-m3
- Papers: 2504.19413, 2310.08560, 2501.13956, 2502.12110, 2402.17753,
  2104.08663, 2402.03216, 2307.06945, 2412.17483
- Anthropic context engineering blog, Claude Code memory docs, pi
  session-format.md, crates.io

## 10. Iterations

- 2026-08-10: initial distillation. Vision, field survey, corpus facts, tier
  architecture, loop architecture, MVP seed (observability + eval).
- 2026-08-10 (second pass): first loop-seed artifact landed, the corpus
  snapshot node. Findings: corpus is English-only; Claude Code writes no
  compaction summaries in this corpus; the distilled compaction layer is
  pi-only and sparse (23 units), so the eval and index must lean on user
  messages as the primary memory unit.
- 2026-08-10 (third pass, review board): four independent reviewer agents
  (correctness, security, architecture, requirements) attacked the doc and
  the node. Verdict: conditional. 6 high / 13 medium / 8 low / 1 note.
  Board artifact: /tmp/code-review/20260810-172613-qol-memory-board-1-526f/
  (temp, not in repo). Key corrections from the board, all applied in the
  hardening pass:
  - stable unit keys (reqs-2), ISO timestamps (correctness-8), honest
    status semantics pass / degraded / fail with per-source errors
    (correctness-7), crash-path and symlink hardening (correctness-9,
    security-5), stratified language sampling (reqs-3), --keep retention
    (security-4), thinking-block counting fixed (arch-4)
  - eval methodology: questions drafted from memory before reading units,
    dev/train split, diagnostic ledger not a gate (arch-5); consumption
    policy for retrieved content deferred to the retriever increment
    (security-3)
  - deferred to Tier 1: store/policy contract (arch-6), weights
    acquisition story (arch-7), daemon ownership (arch-8)
  - environment risk: the WD mount intermittently hides committed files
    (observed by three reviewers and in this session); no data lost, but
    qol setup on the main clone broke because tools/* is a Cargo workspace
    glob and the node lived under tools/. The node now lives in
    docs/research/qol-memory/ next to the doc; main clone workspace is
    green. This feature branch exists because of that break.
  - 2026-08-10 evidence round: the dir reappeared a third time with the
    original 19:03/19:04 mtimes and byte-identical content, confirming
    visibility flake, not recreation: no sync daemon runs, no worktree
    holds the file, dmesg shows no ext4 errors on nvme1n1p1. Root Cargo
    workspace now excludes tools/qol-memory (commit 95b10e84), so stray
    dirs can no longer break cargo metadata regardless of cause.
    Recommended: sudo e2fsck -n /dev/nvme1n1p1 and smartctl -a
    /dev/nvme1n1 at the next maintenance window.
