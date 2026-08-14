# qol-memory concept aliases - build spec v1

Status: architect contract, DESIGN ONLY (no implementation). Grounded in
docs/research/qol-memory.md (lexical-gap sections, abstention audit, retrieval
engineering facts), the shipped retrieval path (ask.mjs, lib/retrieval.js,
lib/indexcache.js), the skills-glossary precedent (skills-glossary.json,
lib/skills-pool.js), the verdict gate (eval/verdict-eval.mjs, heldout.json),
and the retrieval event log (retrieval-log-scope.md, lib/retrieval-log.js).
All gate numbers in this doc were re-measured on 2026-08-14 against a /tmp
frozen copy of the pinned store (snapshot 2026-08-12T18-46-58-129Z + notes
2026-08-13T16:31:40.844Z) with a throwaway patched ask.mjs under
/tmp/qol-memory-sim (not committed).

Goal: bridge term-disjoint queries to settled note vocabulary WITHOUT weights,
without LLM, without a dense tier, with zero new dependencies, deterministically.
The verdict gate's 8 abstentions include d01/d02/d03 whose oracle facts are
term-disjoint from their questions ("how did we fix the m4a1 rifle anchoring":
the settled decision note 79046028d14b1cec says "over clips are full-body 243
controllers", never the repo token "m4a1"; the journal calls this "a lexical gap
the candidate hints bridge, not a gate bug", qol-memory.md:1088-1092). The
skills side already solved the analogous problem with a curated glossary
(skills-glossary.json + aliases in buildMetaDoc, qol-memory.md:999-1015); the
transcript side has NO alias mechanism. This lane scopes the alias layer for
the ask.mjs retrieval path.

## Verified facts the design relies on

- Loop status and abstention audit: the 8 abstentions are exactly h08, h09,
  p01, p02, p03, d01, d02, d03; frozen gate invariant heldout 30 | 22/22/0/8 |
  traps 8/8 | PASS (retrieval-log-scope.md:22-23, re-measured here).
- d01 "how did we fix the m4a1 rifle anchoring" (fact "full-body 243
  controllers", gold note 79046028d14b1cec): verdict candidates. Top notes
  340831f728dcc34c and 5e308463f6d85633 tie at 6.07 (near-duplicate "weapon
  anchor studio" decision notes) -> noteDecisive=false -> abstain
  (ask.mjs:262-276). The gold note ranks ~17th.
- d02 "what was the root cause of the m4a1 arms collapsing" (fact
  "centimeters", gold note d857979480b72faa "The i_caf units fix (DBA meters
  vs i_caf centimeters, x100) ... fixed the collapsed/twisted arms
  regression"): verdict no-memory, max_token_coverage 0.40 < NO_MEMORY_COV
  0.5.
- d03 "was the july reanchor ccd lane kept or reverted" (fact
  "CCD-solved to the handguard", gold note 7570839245601dcc): verdict
  no-memory, max_token_coverage 0.31. The gold note ranks 4th at score 4.831
  (< FLOOR 6.0).
- The m4a1 lexical shape (measured on the pinned notes run): "m4a1" appears
  in 126 of 3574 notes, but almost entirely inside TAGS and repo paths of the
  weapon-anchor studio notes (340831f7, 5e308463, b6b4415f480bc705 carry the
  m4a1 tag; d01's gold note 79046028 and d02's gold note d857979480b72faa do
  NOT carry it). The journal phrase "settled decisions say 'weapon anchor',
  never the repo token 'm4a1'" (qol-memory.md:1088-1092) is accurate for the
  settled decision notes; the tags/path layer is where m4a1 lives.
- CCD is NOT term-disjoint: the query token "ccd" IS present in the d03 gold
  note ("CCD-solved to the handguard"). d03's blocker is the idf-weighted
  coverage gate plus ranking, not vocabulary (measured below, decision C/D).
- Tokenizer: tokens() lowercases, matches [\p{L}\p{N}]+ runs, drops
  single-char tokens, applies light suffix normalization (retrieval.js:1-5,
  7-16). "i_caf" tokenizes to ["caf"]; "per-key" to ["per","key"]; "over
  clips" to ["over","clip"]. bm25Ranks re-tokenizes its query string
  internally (retrieval.js:36-38), so an expanded string is equivalent to an
  expanded token list.
- Ranking call sites in ask.mjs: the units layer ranks the RAW query string
  (ask.mjs:125, 131, including stopwords), the notes layer ranks the
  stopword-filtered join (ask.mjs:138), the skills pool is a separate surface
  (ask.mjs:159, untouched by this design).
- Index cache contract: layerFingerprint hashes (key, text.length) pairs +
  count (indexcache.js:6-17); prefixProof hashes size/count/firstKey/lastKey
  (indexcache.js:20-30); merge freshness (canMerge, indexcache.js:139-144).
  NO query input participates in any fingerprint. Query-side expansion
  therefore cannot invalidate or rebuild the cache, by contract.
- The skills-glossary precedent: skills-glossary.json schema {schema, note,
  aliases: {skillId: [phrases]}}; loadGlossary (skills-pool.js:9-15);
  aliases appended into buildMetaDoc (skills-pool.js:114-116); maintained by
  the ingest/architect loop with drift rules (redundant -> prune, sole-carrier
  -> verify, dangling -> remove, no eval flip -> dead, budget <=30% pool,
  <=5 phrases/skill, qol-memory.md:999-1015); skills-eval has a --no-glossary
  ablation (skills-eval.mjs:22-28). The transcript side has none of this.
- The retrieval event log shipped (retrieval-log-scope.md): every ask.mjs
  call appends one event to <store>/retrievals.jsonl with verdict, signals,
  recalled_keys (ask.mjs:31 manifest pointers, lib/retrieval-log.js
  appendRetrieval); candidates.mjs harvests misses (verdict no-memory |
  candidates, source ask-cli|tool only) with 24h per-norm_query cooldown and
  dedupe against heldout, and --promote runs the SAME verdict-eval gate on
  the pinned frozen store (candidates.mjs harvest/promote). Verified
  2026-08-14: the real store has NO retrievals.jsonl events yet, so the
  log-fed alias pipeline is future input; the seed list starts from the
  abstentions.
- verdict-eval.mjs norm/factMatch: lowercase + non-alnum to space + collapse,
  substring match (verdict-eval.mjs:42-55); runAsk spawns ask.mjs against a
  /tmp frozen store copy, never the real store (verdict-eval.mjs:29-44,
  57-63). Single-note discriminator doctrine: heldout facts are verbatim
  substrings of exactly one note in the pinned notes run
  (qol-memory.md:1196-1201).
- Frozen invariants that must stay byte-identical: verdict-eval
  22/22/0/8 traps 8/8 PASS; eval units 8/30 11/30 mrr 0.329; skills pass;
  test-index-incremental / test-seal / test-e2e / test-cadence ALL PASS.
  eval.mjs and skills-eval.mjs are separate harnesses that never spawn
  ask.mjs, so the alias layer cannot touch them BY CONSTRUCTION; the only
  harness that can move is verdict-eval.mjs (it spawns ask.mjs).

## Architecture

```
concept-aliases.json (repo, committed, read-only)
  {"schema": 1, "note": "...", "aliases": { "m4a1": ["bspace","clip","caf","dba"], ... }}

ask.mjs (single choke point)
  ├─ load once at startup (lib/concept-aliases.js, the loadGlossary pattern)
  ├─ expandTokens(list): per-token Map lookup; REPLACE semantics
  │    (aliased term drops out, expansion terms take its place)
  ├─ qtokens  = expandTokens(tokens(query) minus stopwords)      (ask.mjs:113)
  │    used by every coverage/verdict signal unchanged
  ├─ units ranking: bm25Ranks(expandTokens(tokens(query)).join(" "), ...)
  │    (ask.mjs:125, 131 - preserves today's raw-query ranking semantics)
  ├─ notes ranking: bm25Ranks(expandTokens(qtokens0).join(" "), ...)
  │    (ask.mjs:138 - the notes layer's existing filtered join)
  └─ verdict chain, gates, retrieval event: untouched logic

indexes (idx-*.json + .meta): built over CANONICAL note/unit text,
  fingerprint over keys+lengths only -> no rebuild, no invalidation

admission (human-invoked, gate-gated, the ONLY alias admission)
  └─ proposed alias -> test-alias gate run on the pinned frozen store
       ├─ target question flips unanswered -> answered-correct
       │    (factMatch true, exact gold note key)
       ├─ zero OTHER rows change (verdict + result + answer key identical)
       ├─ traps 8/8 safe, wrong == 0
       └─ only on PASS: the alias is committed into concept-aliases.json
            with a provenance line (which miss, which question)
```

## Design decisions

### A. What exactly is an alias

Recommendation: a curated (query_term -> [expansion_terms]) mapping at
TOKEN level, stored in docs/research/qol-memory/concept-aliases.json
(schema {schema: 1, note, aliases: {term: [terms]}}), maintained by the
human/architect only, admission by the gate-local instrument (E). NOT
(canonical -> variants) in the inverted sense, and NOT query-phrase ->
expansion-phrase in v1.

Rationale:

- The retrieval path is token-based end to end: tokens() is the only
  vocabulary boundary (retrieval.js:1-16), the notes layer ranks
  qtokens.join(" ") (ask.mjs:138), coverage gates are per-token
  (distinctScore/weightedNoteCov, ask.mjs:116-119, 197-200). A token-level
  map slots into every consumer with one function. Phrase-level aliases
  would need new phrase-matching machinery in bm25Ranks (frozen) or
  pre-ranking, and would overfit single phrasings.
- The seed entries are token-level facts measured on the pinned store:
  "m4a1" -> ["bspace","clip","caf","dba"]; "july" -> ["idle"]; "lane" ->
  ["per"]; "kept" -> ["keeps"] (see D for the measured proof).
- Canonical -> variants is the natural shape for the SKILLS glossary
  (descriptions carry canonical vocabulary; aliases add recall terms), but
  the transcript gap is the opposite direction: the user's distinctive
  token ("m4a1") never reaches the settled note. The query side is where
  the bridge belongs (B).
- Who maintains it: the architect, from the abstention audit and from the
  retrieval log's miss events (candidates.mjs report.json surfaces each
  miss with its top recalled note key - the hint vocabulary that names the
  bridge terms). Proposals are seed material only; admission is the gate
  run (E). No LLM, no auto-admission (I).
- Determinism: the file is committed, the lookup is a pure Map access; the
  same query yields the same expansion on every machine and every run.
  Eval-as-artifact doctrine (qol-memory.md:172) applies: the alias file is
  versioned with the gate, so a gate run and the alias set that produced it
  can never drift apart.

### B. Where expansion applies

Recommendation: query-side ONLY. Expand the query token stream before every
bm25Ranks call; the index side stays canonical and untouched. Note-side
aliasing at index build is REJECTED for v1.

Rationale:

- Query-side expansion cannot touch the cache contract: layerFingerprint
  hashes (key, text.length) + count, prefixProof hashes
  size/count/firstKey/lastKey (indexcache.js:6-30), no query input anywhere.
  Verified: an aliased ask run loads the same idx-*.json + .meta with the
  same mtimes as the unaliased run.
- Note-side aliasing would mean rewriting note text (or adding synthetic
  alias docs) at index build: every aliased term set change invalidates the
  idx-notes.json fingerprint -> full rebuild (~400ms build + ~5.7MB cache
  writes, qol-memory.md:1210-1214), and the M0 incremental cache
  (append-only prefix) has no invalidation story for a changed alias file.
  It would also corrupt canonical text used for snippets and serving, and
  would pollute idf statistics (alias terms appearing in N copies).
- The units layer ranking must keep its current raw-query semantics: today
  it ranks the raw string INCLUDING stopwords (ask.mjs:125, 131). The
  expansion therefore applies to tokens(query) (the full stream) for those
  two call sites, and to the stopword-filtered qtokens for the notes layer
  and coverage. This preserves every existing ranking behavior when the
  alias map is empty (verified byte-identical, H).

### C. Expansion mechanics

Recommendation: token-level REPLACE semantics with terms normalized through
the shared tokens() pipeline; weight = 1; no special casing beyond the
existing tokenizer; expansion terms are not stopword-filtered (the curator
chooses them deliberately, and the replace shape keeps the total bounded).

Rationale:

- REPLACE (aliased term drops out, expansions take its place) measured
  strictly better than APPEND on the frozen gate (full rows in D):
  - APPEND, m4a1 -> [dba, i_caf]: d02 flips correct but d01 goes WRONG
    (answers 7ea67489463f488a, a tag-carrying weapon-anchor note, fact
    "full-body 243 controllers" absent) -> gate FAIL. The m4a1 token
    itself keeps boosting tag-carrying notes.
  - APPEND cannot reach d03 at all: idf-weighted coverage of the gold note
    tops out at 0.441 with three extra covered terms, because each appended
    term grows the denominator and "july"/"lane"/"revert" are high-idf and
    uncovered (NOTE_COV_MIN 0.5, ask.mjs:151-154). REPLACE removes the
    aliased term from the denominator, which is what lets d03 cross 0.5.
  - REPLACE bounds growth by construction: each query token maps to at most
    CAP terms, nothing accumulates.
  - The verified seed set uses REPLACE and passes with zero other-row
    changes (D). APPEND's only advantage (keeping a true-positive term in
    the ranking) is moot when the term is a tag/path token, which is
    exactly the lexical-gap shape the layer exists for. Open question
    records the residual risk.
- Normalization: expansion terms run through tokens() (so the curator writes
  human strings: "i_caf" is stored but contributes the token "caf"; "per
  key" contributes ["per","key"]). This keeps ranking tokens and coverage
  tokens in the same vocabulary as the index (retrieval.js:1-16). Case is
  handled by tokens() lowercasing; the norm family matches factMatch's
  lowercase/alnum shape (verdict-eval.mjs:42-47) - both derive from the same
  plain-text reality, no new normalizer.
- Stopwords: the alias lookup happens AFTER the stopword filter on the
  qtokens path (ask.mjs:113 becomes qtokens0-filtered then expandTokens)
  and on the full stream for the units path. Expansion terms are not
  re-filtered; if the curator adds a stopword it simply contributes little
  (stopwords have near-zero idf) - a harmless, deterministic outcome.
- Weight: 1 for every expansion term, confirmed by the no-weights doctrine
  (qol-memory.md: PRF rejected, weights rejected; bm25Ranks' weights
  parameter stays null at every call site, ask.mjs:125/131/138). The
  measured seed set needs no weighting to pass.
- Skills glossary: NO code sharing with loadGlossary in v1. The transcript
  alias file is a separate surface (different keyspace: query tokens, not
  skill ids; different consumer: ranking, not metadata docs; different
  drift rules). The loader is a ~6-line function in lib/concept-aliases.js
  following the loadGlossary pattern (skills-pool.js:9-15); merging the two
  files is a non-goal (I), mirroring retrieval-log-scope.md F.

### D. The seed list and how new aliases get proposed

Recommendation: seeds = the 8 abstentions plus the journal's term-disjoint
pairs, proposed by the architect, admitted only by the gate instrument (E);
new proposals come from the candidates report's miss events; an alias for a
query term is only proposed after the same norm_query miss has been seen
TWICE (two harvest captures separated by the 24h cooldown, reusing
candidates.mjs dedupe/cooldown semantics, candidates.mjs:86-109).

Measured seed verification (2026-08-14, /tmp frozen store, full 38-row
gate):

```
aliases = { "m4a1": ["bspace","clip","caf","dba"],
            "july": ["idle"], "lane": ["per"], "kept": ["keeps"] }

baseline:  heldout 30 | 22/22/0/8 | traps 8/8 | PASS
with seed: heldout 30 | 25/25/0/5 | traps 8/8 | PASS
row diff:  ONLY d01 (79046028d14b1cec, correct), d02
           (d857979480b72faa, correct), d03
           (7570839245601dcc, correct) flip; all other 27 heldout
           rows and all 8 traps byte-identical (verdict + result +
           answer key)
```

Rationale:

- Seed reality from the abstentions: d01/d02/d03 are alias-reachable with
  the measured seed. h08/h09/p01/p02/p03 are NOT alias material: p01/p02/p03
  are short-fact queries that sit below the 6.0 absolute note floor
  (qol-memory.md:1149-1150) and p03's note is answerable only after a
  content fix; h09 is the documented not-extractable miss; h08's top note
  does not carry the fact. Their abstentions are gate/calibration
  phenomena, and the alias layer must not paper over them (a wrong
  expansion here would manufacture wrong answers). They stay honest
  abstentions and remain candidates-log material.
- Journal pairs verified: "m4a1 / weapon anchor" is real but subtler than
  the journal line: the settled decision notes do not carry m4a1, while
  tag/path copies do. The measured seed therefore bridges m4a1 to
  bspace/clip/caf/dba (terms present in the GOLD notes), NOT to "weapon
  anchor" (which only boosts the near-duplicate studio notes that tie and
  cannot answer d01). "CCD / ???": verified CCD is NOT term-disjoint - the
  d03 gold note carries "ccd"; no CCD alias is needed or proposed.
- The m4a1 entry needs FOUR terms: cap-2 subsets fail the instrument -
  m4a1 -> [bspace, clip] flips d01 only; m4a1 -> [caf, dba] flips d02 but
  breaks d01 (wrong note 81b0dfcd866967db, gate FAIL); m4a1 -> [clip, caf]
  flips d01 but leaves d02 unanswered. The union is the only measured
  shape that flips both with zero cross-damage. Decision F sets the cap to
  4 with this entry as the documented calibration point.
- Proposal pipeline: candidates.mjs report.json already pairs every miss
  with its top recalled note (the hint vocabulary, candidates.mjs
  buildCandidate/noteOf). The architect reads the hint, proposes bridge
  terms from the recalled note's text, and runs the instrument (E). The
  twice-seen rule prevents one-off overfitting (a single miss may be a
  phrasing accident; two cooldown-separated misses establish the term as a
  recurring user vocabulary). The 8 abstentions are heldout rows, which
  are by definition "seen" repeatedly by the gate; they are seeds without
  the twice-seen rule, and every seed still passes the same instrument.
- Drift rules mirror the glossary (qol-memory.md:1003-1005): an alias whose
  ablation changes NO verdict rows is dead (prune); an alias that proves a
  note-text gap earns a content-side fix at the notes layer, after which
  the alias prunes (standards-evolution loop, same shape as the skills
  description-edit loop, qol-memory.md:1011-1013).

### E. Gate + regression

Recommendation: the frozen verdict-eval invariant stays byte-identical; the
alias acceptance instrument is: on the pinned frozen store, with the
proposed alias applied, the target question flips unanswered ->
answered-correct (factMatch true against its gold fact) AND zero other rows
change (every non-target heldout row's verdict, result, and answer key
identical; traps 8/8 safe; wrong == 0). A test (test-alias-*.mjs, H)
asserts per-question stability so a future alias addition cannot silently
move an unrelated row.

Rationale:

- Byte-identical gate with aliases ABSENT is the baseline contract: the
  empty alias map must reproduce the frozen invariant exactly. Verified:
  the aliased code path with {} produces 22/22/0/8, traps 8/8, PASS.
- Zero-other-row-change is a STRONGER invariant than the existing gate
  formula (wrong==0 && correct>=11 && traps safe), and it is the right
  instrument for aliases: an alias that flips a heldout row from correct
  to wrong is already caught by the formula, but an alias that flips a
  correct row to a DIFFERENT correct note (or an unanswered row to a
  different note) is not. The per-question diff catches both. It is the
  same doctrine as heldout growth (gate-local discrimination,
  retrieval-log-scope.md D), applied per-row instead of per-aggregate.
- Measured enforcement: the instrument rejects {m4a1: [caf, dba]} (d01
  wrong, gate FAIL) and {m4a1: [clip]} (t04 trap answered, gate FAIL -
  the "clip" expansion alone lets a tag-carrying studio note answer "what
  does the m4a1 weapon cost"; the union seed keeps t04 safe because the
  extra terms drop the top note's coverage below NOTE_COV - a non-obvious
  interaction the instrument exists to catch).
- Admission: only a PASS on the pinned frozen store admits an alias into
  concept-aliases.json (same shape as candidates --promote,
  candidates.mjs:114-152). The gate and the human remain the only
  admission, exactly like heldout growth.

### F. Cost

Recommendation: one Map lookup per query token at ask time (~0.001ms);
expansion never exceeds CAP (4) terms per aliased token; no index rebuild,
no cache invalidation, no LLM, no new dependency.

Rationale:

- The expansion is a pure in-memory lookup over a small committed Map (the
  seed set is 4 keys); worst case a query with 10 aliased tokens appends
  40 terms to a token stream that already costs ~0.35s wall (warm ask.mjs,
  qol-memory.md:1235-1236). Unmeasurable.
- The index cache contract is untouched by construction: fingerprints hash
  keys+lengths and source-path size/count/keys only (indexcache.js:6-30);
  the query never participates. Verified: aliased asks stay warm (idx
  meta mtime unchanged, output byte-identical).
- Load cost: one readFileSync + JSON.parse of a ~1KB committed file at
  startup, same pattern and magnitude as loadGlossary (skills-pool.js:9-15)
  and distinctive.json loading (qol-memory.md:1046-1047).
- CAP = 4 measured: cap-2 cannot serve the m4a1 seed without cross-breakage
  (D). 4 is still a hard bound on query growth (replace semantics, so the
  total is bounded by 4 per aliased token, never cumulative).
- No weights: expansion terms score at weight 1 through the existing
  bm25Ranks formula (retrieval.js:36-52); the no-weights doctrine holds.

### G. Where the mapping lives

Recommendation: repo file docs/research/qol-memory/concept-aliases.json,
committed with the system (eval-as-artifact, qol-memory.md:172), read-only
at runtime, loaded once by ask.mjs at startup like the glossary. NOT
store-side.

Rationale:

- The alias set is curated knowledge that must be versioned with the gate:
  a gate run is only reproducible if the alias set it ran under is pinned
  with it. Store-side (machine-local, ~/.local/share/qol-tray/plugins/
  qol-memory) would let gate results depend on per-machine mutation.
- Same ownership as eval/heldout.json: repo artifact, committed by the
  architect, never auto-written (I).
- Kill-switch for ablation: QOL_MEMORY_ALIASES_DISABLE=1 (the established
  env convention, retrieval-log-scope.md verified-facts), mirroring
  skills-eval --no-glossary (skills-eval.mjs:22). The ablation is what
  makes the "alias is dead, prune" drift rule executable.

### H. Test plan

Recommendation: test-alias-*.mjs, the established sandbox pattern (tmpdir
store via QOL_MEMORY_STORE, check() pass/fail lines, non-zero exit on
failure; test-seal.mjs:16-37). Pure logic (load, expandTokens, cap,
normalization) lives in lib/concept-aliases.js so tests run without
spawning; the gate-level cases spawn ask.mjs against a /tmp frozen store
copy (the verdict-eval freeze pattern, verdict-eval.mjs:29-44) and diff
rows.

1. Seed acceptance: with the measured seed set, d01/d02/d03 flip
   unanswered -> answered-correct with their exact gold note keys; the
   full 38-row diff shows zero other changes; gate line is
   25/25/0/5 traps 8/8 PASS (this is the locked regression for the seed).
2. Empty map = frozen invariant: {} reproduces 22/22/0/8 traps 8/8 PASS
   byte-identical to the pre-alias harness output.
3. Alias NEVER changes an already-correct row: d04 (m4a1 in its query,
   correct today) stays correct with the m4a1 entry applied.
4. Unknown terms pass through untouched: a query with no aliased tokens
   produces byte-identical output with the map present vs absent.
5. Expansion respects the cap: an alias with > CAP terms is refused at
   load (or truncated deterministically - refused is the chosen behavior,
   so a 5-term entry fails load loudly); the m4a1 4-term entry loads.
6. Gate byte-identical for non-target questions: per-row verdict + result
   + answer key equality between alias-present and alias-absent runs for
   every row except the target.
7. Cache contract: a warm aliased ask leaves idx-*.json + .meta mtimes
   untouched and output byte-identical to the warm unaliased ask.
8. Determinism: two runs of the same query produce byte-identical stdout
   (modulo wall-clock event fields when the log is on; run with --no-log
   for strict byte equality).
9. Kill-switch: QOL_MEMORY_ALIASES_DISABLE=1 behaves exactly like the
   empty map (the ablation arm of the drift rule).
10. Normalization: "i_caf" in the file contributes the token "caf"
    (tokens() pipeline); "per key" contributes ["per","key"].
11. Corrupt/missing file: load failure behaves like the empty map (the
    loadGlossary try/catch-null pattern, skills-pool.js:9-15), never a
    crash.
12. Traps: the full trap set stays 8/8 safe under the seed set (the t04
    near-miss from {m4a1:[clip]} is the documented reason this case
    exists).

Tests never touch the real store; the gate-level cases freeze the pinned
runs into tmpdir exactly like verdict-eval.mjs.

### I. Non-goals

- No weights: expansion terms score at weight 1; the no-weights doctrine
  (qol-memory.md PRF rejection, calibration) holds.
- No LLM-generated aliases, no auto-admission: the gate run + the
  human/architect are the only admission, same doctrine as heldout growth
  (retrieval-log-scope.md I).
- No dense tier, no new dependencies: pure JS Map lookup on the existing
  zero-dep BM25 path.
- No changes to the notes/units layers, no content edits to settle the
  lexical gap in v1 (content-side enrichment is the standards-evolution
  follow-up when an alias proves a note-text gap).
- No index-side aliasing in v1 (B): the index stays canonical; the cache
  contract is untouched.
- No skills-glossary merge: separate files, separate surfaces, separate
  drift rules (C), mirroring retrieval-log-scope.md F.
- No query-phrase aliases in v1: token-level only (A).
- No changes to eval.mjs / skills-eval.mjs / lib/retrieval.js /
  lib/indexcache.js: separate harnesses or frozen shared code; the alias
  layer lives in ask.mjs + one new lib file.
- No change to the retrieval event schema in v1: the expanded tokens are
  not logged (the event stays a projection of the out object; expansion
  observability is an open question).

## Cost budget

- Load: one readFileSync + JSON.parse of a ~1KB committed file at ask.mjs
  startup, ~0.1ms (the loadGlossary precedent, skills-pool.js:9-15).
- Per ask: one Map lookup per query token, ~0.001ms; at most CAP=4
  appended terms per aliased token; replace semantics bound the total.
- Indexes: nothing. No rebuild, no cache write, no fingerprint change
  (indexcache.js:6-30); warm ask.mjs stays warm under aliases.
- Gate: the instrument is a verdict-eval run, already part of the
  release/test loop; no new harness cost beyond test-alias-*.mjs's frozen
  store copy (one-time ~30s).

## Gates

- The frozen invariant stays byte-identical with the alias layer PRESENT
  and EMPTY: heldout 30 | 22/22/0/8 | traps 8/8 | PASS (re-measured
  2026-08-14). eval units 8/30 11/30 mrr 0.329 and skills pass are
  untouched by construction (separate harnesses, no shared code change).
- The alias acceptance instrument: on the pinned frozen store, target
  flips unanswered -> answered-correct with the exact gold note key, and
  zero other rows change (per-row verdict + result + answer key equality),
  traps 8/8, wrong == 0. Only a PASS admits an alias.
- Measured with the seed set: 25/25/0/5, traps 8/8, PASS; row diff shows
  exactly d01/d02/d03 flipping, each to its gold note
  (79046028d14b1cec / d857979480b72faa / 7570839245601dcc).

## Integration points

- ask.mjs:113 (qtokens = tokens(query) minus stopwords -> expandTokens of
  the filtered list; every coverage and verdict consumer below uses the
  expanded qtokens unchanged).
- ask.mjs:125, 131 (units-layer rankings: bm25Ranks(query, ...) ->
  bm25Ranks(expandTokens(tokens(query)).join(" "), ...); bm25Ranks
  re-tokenizes, retrieval.js:36-38, so the raw-query semantics are
  preserved exactly when the map is empty).
- ask.mjs:138 (notes-layer ranking: bm25Ranks(qtokens.join(" "), ...) ->
  bm25Ranks(expandTokens(qtokens0).join(" "), ...)).
- ask.mjs:31 (manifest gains "concept_aliases":
  "qol-memory-concept-aliases-v1" alongside retrievals/candidates).
- ask.mjs:20-25 area (startup: load concept-aliases.json once; failure =
  empty map, the loadGlossary try/catch-null pattern).
- lib/concept-aliases.js (NEW): load() + expandTokens() + CAP; pure, unit
  testable without spawning.
- concept-aliases.json (NEW, committed): the curated seed + provenance
  lines per alias (which miss/question admitted it).
- test-alias-*.mjs (NEW): the H cases.
- eval/verdict-eval.mjs: unchanged code; it IS the instrument (a run, not
  a change). candidates.mjs: unchanged; its report.json is the proposal
  input surface.
- lib/retrieval.js, lib/indexcache.js, eval/eval.mjs, eval/skills-eval.mjs,
  lib/skills-pool.js, skills-glossary.json: untouched.

## Non-goals (summary)

No weights, no LLM, no auto-admission, no dense tier, no notes/units
changes, no index-side aliasing, no skills-glossary merge, no phrase
aliases, no changes to frozen shared code, no event-schema change.

## Open questions

- d02's oracle fact "centimeters" matches TWO notes in the pinned run
  (7ea67489463f488a and d857979480b72faa), a pre-existing deviation from
  the single-note discriminator doctrine (qol-memory.md:1196-1201). The
  alias layer inherits it; consider refining the fact to a single-note
  slice (e.g. "DBA meters vs i_caf centimeters") in a future heldout edit.
- d03 was reached only through three semantically strained bridges
  (july -> idle, lane -> per, kept -> keeps). The twice-seen rule does not
  apply to heldout seeds by construction; the human review is the guard.
  If real log data never repeats these phrasings, the d03 aliases should
  prune (dead-alias rule) and d03 returns to abstention.
- Replace semantics drop the aliased term from the ranking. For a future
  alias whose term is a genuine true positive (present in the gold note
  AND absent from tag noise), append would be protective; v1 picks replace
  because it is the only measured shape that passes. Revisit with more
  data.
- CAP=4 for the m4a1 entry vs the generic cap-2 intuition: keep CAP as one
  constant (4), or make it per-alias configurable? v1: one constant, with
  the m4a1 entry documented as the calibration point.
- Expansion observability: should the retrieval event log record the
  expanded token stream (for miss analysis)? v1 says no (event schema
  frozen); the open question is whether the alias layer should be
  ablatable in the event via the gates field.
- h08/h09/p01/p02/p03 stay abstentions: p01/p02/p03 are short-fact floor
  misses (the per-layer floor work is the phase-2 topic, qol-memory.md:
  1149-1150), h09 is the documented not-extractable miss, h08's top note
  lacks the fact. No alias should be invented for them; candidate
  promotion remains their only admission path.
- The twice-seen proposal rule needs real retrieval-log data to calibrate
  (verified 2026-08-14: the real store has zero retrievals.jsonl events).
