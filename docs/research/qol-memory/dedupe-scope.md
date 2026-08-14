# qol-memory dedupe - research scope v1

Status: Round 1 measurement + design research for the duplicate-information
optimization, grounded on real-store measurements (read-only, 2026-08-13).
Architect reviews; implementation gated on this scope. Every number below was
measured on the live store (~/.local/share/qol-tray/plugins/qol-memory) with
measurement scripts in /tmp/qol-mem-measure (measure.mjs, measure2.mjs,
measure3.mjs, measure4.mjs, fambreak.mjs).

Goal: decide whether, and how, to collapse duplicate information in the
append-only units.jsonl into small hash references, under three hard
constraints: append-only doctrine (never delete or rewrite existing units; the
d01 loss, docs/research/qol-memory.md 2026-08-13 board review, destroyed 11
decision notes via a 0.25 token-containment drop and the fix was exact-key
dedupe only), the read-path performance mandate (10x growth must not breach
the 6s tool timeout; live-capture append stays 0-1ms), and zero new
dependencies.

## Measured facts (2026-08-13, real store)

units.jsonl: 4,066 lines, 16,560,766 bytes. kinds: user 4,014, compaction 52.
Average user unit 3,471 bytes. Read path: readFileSync 38.8ms + JSON.parse
30.3ms + split = 75.5ms, matching the 75ms baseline in the mandate.

ask.mjs end-to-end (on a /tmp copy of the store, same units): cold (no caches)
0.96s, warm (fresh caches) 0.38s, stale (one unit appended since the cache)
1.01s, peak RSS 196-253MB. The stale case is the norm: live capture appends
~220 units/day, and the cache fingerprint covers every unit key + text length,
so any append invalidates both caches.

Index rebuild on a stale read: buildIndex 408.6ms over 4,008 deduped user
units, layerFingerprint 1.5ms, saveIndex 62.7ms, ~5.7MB of cache written per
stale call (idx-pool.json 1,623,534B N=3,145 + idx-user.json 4,079,382B
N=3,948; idx-notes.json 707,601B). 12 stale idx-pool-x-*.json files (~1.5MB each, ~16MB total) accumulate per excluded session and are never pruned.
Tokenizing all units: 0.37s. sha256 of normalized text: 34us per unit. norm():
29us per 2.6KB text.

## Measurement table

| metric | value |
| --- | --- |
| exact dup units (normalized text seen earlier) | 6 / 4,066 (0.15%), 1,202B (0.01% of text bytes) |
| exact dup group sizes | 4 groups of 2, 1 group of 3 |
| simulated exact-ref savings today | 932B (0.01% of user bytes) |
| unique user norms vs lines | 4,008 / 4,014 (read-time dedupe already yields 4,008) |
| multi-member first-12-token families | 53 families, 1,007 / 4,014 units (25.1%) |
| family byte share | 10,796,746B = 77.5% of user bytes |
| units sharing first-12 tokens with an earlier unit | 954 / 4,014 (23.77%) |
| units sharing a first-200-char prefix | 517 / 4,014 (12.88%) |
| near-dup, sampled full scan (313 samples), max token-Jaccard vs any earlier unit >= 0.8 | 21 / 313 (6.71%) |
| same at >= 0.7 / >= 0.5 | 32 (10.22%) / 68 (21.73%) |
| near-dup, family-bucketed full scan (112,109 pairs), >= 0.8 | 835 / 4,014 (20.80%) = 75.18% of bytes |
| same at >= 0.9 / >= 0.95 | 18.93% / 73.72% bytes; 17.91% / 72.06% bytes |
| distinct-token delta of >= 0.8 pairs | avg 243, median 79, max 5,568 |
| near-dup >= 0.8 units average size | 12,544B vs 3,471B corpus average |
| clause-prefix refs (norm LCP >= 200 vs family first) | 309 refs, 372,098B (2.67%) |
| same, pairwise-best LCP (upper bound) | 513 refs, 570,147B (4.09%) |
| k=32-mer clause coverage vs family-first template | 1,153,195B of 10,367,264B later bytes (11.1%) = 8.3% of all user bytes, 954 candidate refs |
| simhash 64-bit (0.3s for all units, 75us/unit) Hamming <= 4 | 84.5% candidate rate, 14% precision@0.8, 56% recall@0.8 |
| same, Hamming <= 6 / <= 8 | 94.5% / 18% / 81%; 97.1% / 19% / 89% |
| newest notes run (2026-08-12T20:33:52.371Z) | 3,522 notes: path 3,242, decision 153, policy 45, model 30, command 29, config 7, flag 5, status 4, count 3, format 2, version 1, unitkey 1 |
| notes cross-run exact dups | 3,497 / 3,522 (99.3%) |
| notes within-run fam repeats | 175 / 3,522 (5.0%) |
| note window verbatim in some units.jsonl unit | 1,572 / 3,494 (45.0%) |
| last 500 user units in multi-member families | 248 / 500 (49.6%), 68.1% of recent bytes |

Family detail (byte share, collapsible share): review-this-change x410 (50.5%
of user bytes, LCP-vs-first only 118 chars, k-mer coverage 2.8% - each member
is ~97% unique content), continuation x105 (13.5%, coverage 5.1%), bridge
x195 (3.2%, coverage 14.4%), adversarial-verify x76 (2.1%, coverage 75.4%),
dup-finding x28 (0.5%), rust-review x14 (0.3%, coverage 96.5%), skill-context
x10 (coverage 94.3%), sessions-close x22, bridge-previous x14.

The 835 near-dup >= 0.8 units sit in 44 families (zero singletons): review
x359, bridge x127, continuation x101, adversarial x76, dup-finding x19,
bridge-previous x14, rust-review x14, skill-context x10, plus 36 smaller
families. 620 / 835 are boilerplate-marker units. Token Jaccard overstates
byte redundancy (function-word-heavy vocab); the k=32 byte-level ceiling is
the number that matters.

Growth composition: the last 500 user units are 49.6% family members (68.1%
of recent bytes), up from 25.1% all-time - template-family prompts dominate
the append stream and the collapsible share of new bytes depends on which
families recur (review family: 2.8% collapsible; adversarial/skill/rust
families: 75-97%).

## Exact-tier design (the in-place ref)

Format: the first occurrence of a normalized text stores the full unit plus
h: <normsha16>; every later occurrence stores the same line position with its
own key/source/file/session/cwd/kind/ts but text replaced by ref: <normsha16>.
normsha16 = sha256(normalized text).slice(0, 16), a content hash. The unit key
cannot be the pointer (it includes ts and differs per occurrence); the ref
target is the stream-first occurrence of that norm. Refs are exact-match
only, so the merge is reversible and information-preserving by construction:
the original line is never deleted or modified, and expansion restores the
original text.

Where the collapse runs: at ingest time, not append time. ingest.mjs already
rewrites the whole file (read all, concat, write all) on every run, so the
collapse is a byproduct of that existing rewrite, idempotent, and needs no new
knowledge in the write path. The live-capture extension stays untouched and
its 0-1ms guarantee stays trivial. An append-time variant (extension emits the
ref) would require a norm-to-key sidecar loaded at extension startup plus
~40us per append (measured sha256+norm), and is a later refinement, not v1.

Reader inventory (every consumer of units.jsonl, and what each needs):

| reader | reads units.jsonl? | ref handling |
| --- | --- | --- |
| ask.mjs readUnits + dedupeUserUnits + buildOrLoad | yes | must expand refs before dedupe/index/snippet/boilerplate checks; single forward pass builds normsha-to-text from h fields, refs resolve O(1); no hashing at read time (h is stored) |
| ingest.mjs mergeStep | yes | keys only; refs survive parse/serialize unchanged; the collapse pass is the only new logic |
| qol-memory-tool.ts (shipped ext, qol-skills) | no (append-only writer) | unchanged in the ingest-time variant |
| snapshot.mjs | no (writes snapshot runs) | unaffected |
| replay.mjs / recall-new.mjs | no (snapshot runs) | unaffected |
| notes.mjs | no (snapshot runs) | unaffected |
| decisions.mjs | no (snapshot + notes runs) | unaffected |
| distinctive.mjs | no (idx-notes.json) | unaffected |
| eval/eval.mjs, eval/skills-eval.mjs | no (snapshot runs / skills index) | unaffected |
| calibrate.mjs | via ask.mjs only | affected only through ask.mjs |
| eval/verdict-eval.mjs | guards against units.jsonl in the frozen store (throws if present) | keep that invariant; refs can never enter the frozen eval store |
| lib/indexcache.js | via items passed to buildOrLoad | ref-agnostic once ask.mjs expands before calling |

Order preservation: the ref line occupies the same position in the stream as
the full line it replaces, and keeps its own ts/session/key, so append order
is byte-identical and the ts sort in dedupeUserUnits is unaffected. One
documented nuance: the ref points at the stream-first occurrence while
read-time dedupe keeps the ts-first survivor; within a dup group the raw texts
are norm-equivalent, so retrieval (tokens, snippets) is unchanged.

Savings: 932B today (0.01%), ~10KB at 10x if the dup share stays constant.
What breaks: (a) the invariant key = sha256(source|file|ts|text) no longer
holds for ref lines, so the live-capture key-parity test (live-capture-scope
section 1b) must scope to full units; (b) dedupeUserUnits would collapse refs
to the empty norm if expansion did not run first; (c) any future direct reader
of units.jsonl must resolve refs (today: ask.mjs and ingest.mjs only). What is
already free: ask.mjs read-time dedupe (dups never indexed: N=3,948 vs 4,014
lines), snapshot.mjs run-local dedupe, ingest key-dedupe, notes.mjs
normalized-text dedupe, decisions.mjs exact-key carry (post-d01). Exact dups
cost retrieval nothing today; they cost 0.01% of file bytes.

## Semantic-tier feasibility (the O(1) near-duplicate map)

Measured candidates, all zero-dep, all sub-ms amortized per unit:

- Family prefix bucket (first-12-token norm hash): the O(1) map that actually
  exists. 53 buckets cover 77.5% of user bytes; lookup is one hash per append
  (~40us measured norm+sha). It is a perfect family-membership gate and is a
  byproduct of the ingest read pass.
- Simhash 64-bit random projection: 75us per unit to sketch, banded O(1)
  candidate lookup. Measured on real pairs: Hamming <= 4 catches 56% of
  Jaccard >= 0.8 pairs at 14% precision; Hamming <= 8 catches 89% at 19%
  precision. Candidate lists are dominated by low-similarity family members,
  so the sketch is a candidate generator, never a merge decision. (An initial
  sketch weighting produced degenerate bits and was discarded; the corrected
  weighting is the one in the table.)
- MinHash k=16/32: same cost class, same candidate-generator role, no
  advantage on this data.
- n-gram inverted index: equivalent candidate generator, strictly worse for
  merge safety (same delta problem below).

Doctrine check: any near-dup merge must be reversible, and the original stays.
But a whole-unit ref to a >= 0.8 Jaccard twin destroys a median of 79 distinct
tokens (avg 243, max 5,568) of real content - those tokens are the finding,
the task body, the diff. That is the d01 loss recurrence: a similarity-based
merge that drops information. The only safe semantic tier is clause-level and
lossless: extract the shared substrings against a family template and keep the
remainder (ref + span list + remainder, exactly reversible). The measured
ceiling for that is the k=32-mer coverage: 8.3% of all user bytes today
(~1.15MB, 954 candidate refs), reachable with the family bucket + a rolling
hash k-mer index (~60 lines, zero deps), with no sketch machinery at all.

Verdict: the semantic tier as specified (O(1) near-dup map driving a merge) is
not worth it. Measured precision at the 0.8 gate is 14-19%, whole-unit merge
is doctrine-unsafe, and the safe lossless variant reaches the same ceiling
through the family bucket alone. The user-visible "semantic O1 map" is
satisfied by the family bucket; the simhash/minhash/ngram options add
complexity without adding capture.

## Recommendation (MVP sequence)

1. M0 - incremental index cache (the actual 10x fix). Measured: the stale-cache
   ask.mjs call costs 1.01s today, of which parse is 76ms and the full index
   rebuild is ~470ms + 5.7MB of writes; at 10x the rebuild projects to ~4-5s
   plus ~1.2s save plus ~760ms parse, about 6-7s total, breaching the 6s tool
   timeout. The rebuild, not the parse, is the breach driver, and dedupe does
   not touch it. Change indexcache.js only: persist per-term df + doc rows +
   N, fingerprint on (last key, count), append new rows incrementally,
   recompute idf from df on load (the formula already lives in
   lib/retrieval.js). Stale read cost drops to warm-equivalent (~0.4s) and
   stays flat at 10x. Also prune the stale idx-pool-x-* files (~16MB,
   unbounded per excluded session). Zero store-format change, zero doctrine
   risk.
2. M1 - exact-tier refs (the specced design), ingest-time collapse + ask.mjs
   expansion. 0.01% of bytes today; doctrine-safe; ships the ref format
   primitive that M2 builds on. Cheap, but on its own it does not move the 10x
   needle.
3. M2 - clause-tier refs (ref + span list + remainder against the family
   template), ingest-time only, zero-dep rolling-hash k-mer extraction.
   Measured ceiling 8.3% of user bytes today (~1.15MB), lossless and
   reversible, projected ~11.5MB at 10x if the share holds. Gate M2 on the
   growth-composition question: the last-500 sample shows family share rising
   (49.6% of units) but the dominant family is only 2.8% collapsible, so the
   true 10x capture is uncertain (range 3.7-11.5MB from the measured
   prefix/k-mer bounds).
4. Semantic tier (simhash/minhash/ngram merge map): no. Unsafe (median 79
   tokens destroyed at the 0.8 gate), low precision (14-19%), and dominated by
   the family bucket + k-mer clause extraction.

Concrete first implementation round if greenlit: M0 only (indexcache.js
incremental layout + stale pool-x pruning + a warm-path timing test on a /tmp
store copy), then M1 as a format round, M2 re-scoped on fresh growth
measurements. Acceptance for M0: stale ask.mjs on a /tmp copy with one new
unit stays within ~0.4s at 4k units and projects under 2s at 10x (simulated
by a synthetic 40k-unit copy), frozen evals unchanged, real store untouched
except by ingest runs.

## Test bar for any greenlit round

1. Append 0-1ms: extension handler unchanged (M0/M1/M2 all ingest-time).
2. Order preservation: after a collapse run, line positions, ts order, and
   keys of every non-collapsed unit are byte-identical; collapsed lines carry
   their own ts/session/key.
3. Expansion fidelity: every ref resolves to a text whose normsha equals the
   ref; a random sample of expanded texts matches the pre-collapse text
   modulo whitespace/case.
4. Frozen evals unchanged: eval.mjs (pinned run) and verdict-eval.mjs gate
   identical to before (frozen store never carries units.jsonl).
5. Doctrine: the collapse pass never deletes or edits a first-occurrence line;
   re-running the pass is a no-op (idempotent).
6. Real-store sanity: after an ingest run, units.jsonl stays valid JSONL, and
   ask.mjs --brief answers the same questions before and after (spot-check
   heldout queries on a /tmp copy).

## Out of scope

- Append-time ref emission by the shipped extension (sidecar norm map); v1
  keeps the extension untouched.
- Compressing snapshot runs, notes runs, or decisions runs (separate
  append-only artifacts; cross-run notes redundancy at 99.3% is deliberate
  per-run versioning).
- Any similarity-based drop or merge of whole units (forbidden by the d01
  doctrine).
- Dense embeddings or learned similarity (zero-dependency mandate).
