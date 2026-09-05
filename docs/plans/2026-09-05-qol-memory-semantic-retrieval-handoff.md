# QoL Memory semantic retrieval handoff

## Objective and user expectations

Deliver general semantic question answering over recorded memories for humans
in the launcher and agents through the headless interface. Relevant memories
are already retrieved in the user's examples; the failure is recognizing when
their evidence answers the question.

The user explicitly rejected command-specific fixes. KCD2 launch commands are
examples of a general semantic failure, not the scope of the feature. The user
also accepted that a vague topic such as `kcd2 retail` can reasonably show
related memories. An explicit question such as `how to launch kcd2 retail`
should use the explanation that connects retail to the recorded normal-mode
launch command.

The user is frustrated by hours of work followed by little visible improvement.
Do not equate narrow fixture success, code shipped, or an optional feature with
delivery of the intended behavior. Report exactly what works in the default
launcher flow and what remains broken. Do not keep adding synonyms or lower a
confidence threshold to make individual screenshots pass.

Automatic capture triggers, harness hooks, and memory coalescing were part of
the original broader request. They remain subsequent work; semantic answer
selection is the current unresolved priority.

## Current implementation and delivery

These commits are on main; consult current Git history before relying on hashes
because other sessions have rewritten earlier local history:

| Commit | Delivered behavior | Limitation |
| --- | --- | --- |
| `ee7e6537e` | Optional asynchronous local answer verifier, derived bindings, launcher polling and stale-response protection | Disabled by default; model policy failed its accuracy gate |
| `d4c0551f9` | Deterministic shorthand matching, boot vocabulary, explicit command-alternative agreement | Lexical question matching is still not semantic understanding |
| `dc36a90ce` | Compare complete recorded answers instead of just their first sentences | Conservative textual disagreement can still reject equivalent explanations |

The latest Memory binary was built, loaded through the tray's single-plugin
reload owner, and verified against the tested executable hash. The running
daemon still answered `kcd2 debug`. Deployment evidence is in the repair report
below. Process IDs are transient; do not reuse them as a restart mechanism.

`verify_answers` remains false by default. No setting was enabled for the user.
The existing local provider uses a pinned `qwen3:8b` model through Ollama. Merely
enabling it is not a demonstrated solution. See the prior implementation plan
for provider requirements and the failed qualification evidence.

## Root causes established from source and experiments

1. Retrieval and answer selection use different evidence. Retrieval finds
   relevant full text. Deterministic selection chiefly matches extracted stored
   questions, with a small vocabulary and token-matching rules.
2. Legacy evidence extraction can discard the explanatory suffix from the
   recorded answer. In the retail example, the stored question says normal mode
   and the suffix explains retail. The semantic candidate builder also receives
   the extracted question/answer, so that missing context affects the optional
   verifier too.
3. Semantic candidate retrieval ranks extracted questions and accepts only
   captures from which the question/answer parser can extract evidence. Useful
   declarative captures may be absent from that candidate pool.
4. Subject and scope matching are incomplete. The simulation accepted a
   Linux-only command for a platform-unspecified question and selected an RCON
   answer for an ambiguous retail topic.
5. Agreement is not semantic equivalence. Differently worded Rust answers can
   be treated as conflicts. The latest fix prevents a shared opening sentence
   from hiding contradictory details, but does not classify those details.
6. `NO CONFIDENT ANSWER` conflates missing support, ambiguous intent, conflicting
   evidence, and failure of the matcher to understand available evidence.
   Answer rows also have a `true now` trail tag. Revisit that presentation for
   historical, conditional, speculative and explicitly unverified records.

The latest agreement fix retains the complete *extracted recorded answer*.
It does not repair legacy extraction's omission of explanation after the
recorded question. Do not mistake it for preservation of the entire memory.

## Source map

| Owner | Files |
| --- | --- |
| Question parsing, evidence extraction, answer agreement | `plugins/qol-memory/src/ask/question_match.rs` |
| Candidate agreement and conflict selection | `plugins/qol-memory/src/ask/selection.rs` |
| Retrieval and answer policy | `plugins/qol-memory/src/ask/mod.rs` |
| Semantic candidate preparation and application | `plugins/qol-memory/src/ask/semantic.rs` |
| Background verification, provider and bindings | `plugins/qol-memory/src/verification/` |
| Daemon requests, warm state and invalidation | `plugins/qol-memory/src/app/request.rs`, `warm.rs` |
| Launcher rows and provenance presentation | `plugins/qol-memory/src/ask/rows.rs` |
| Launcher async interaction | `plugins/launcher/src/flow/mod.rs`, `src/ui/{controller,state,view,render}.rs` |
| Selection regressions | `plugins/qol-memory/tests/answer_selection.rs` |
| Comparison workflows | `plugins/qol-memory/scripts/evaluate.mjs`, `scripts/comparison/` |
| Headless comparison workers | `plugins/qol-memory/examples/matcher-{baseline,runtime,verifier}.rs` |

Existing probes include `QOL_MEMORY_DAEMON` rows verdict/match/conflict counts
and verification states, plus `LAUNCHER_FLOW`. Enrich existing probes instead
of introducing another tracing channel. Do not log memory text in new probes.

## Measured state and evidence

All report paths below are relative to the repository root. Reports and temporary
scripts are local artifacts and may not be present in another checkout. Preserve
or transfer them before delegating to an environment without this workspace.

### General confidence simulation

Baseline:

`reports/qol-memory/confidence/2026-09-05T17-51-40.358Z/assessment.md`

The adjacent `cases.json` and `report.json` contain all 24 independent synthetic
scenarios, evidence, expected decisions, actual decisions and artifact identity.
The expected behavior was agent-authored; it is diagnostic evidence, not an
independent accuracy benchmark or calibrated confidence score.

Seventeen cases have binary answer/withhold expectations. Seven require
qualification and were intentionally not scored as binary decisions. Cold ask,
warm ask and the actual launcher rows handler agreed. No desktop UI was used
for this simulation.

After the complete-answer agreement fix:

`reports/qol-memory/confidence/2026-09-05T17-55-43.168Z/repair-report.json`

- Binary decision matches improved from 9/17 to 10/17.
- The shared-first-sentence contradiction changed from answered to candidates.
- The other 23 scenarios were unchanged.
- Seven binary semantic failures remain: ordinary paraphrase, evidence in the
  explanation, vague-topic overconfidence, omitted platform scope, equivalent
  answers with different wording, version-specific evidence, and unit conversion.
- Qualification cases cover retain/lose polarity, explicit negation, conditions,
  an explicitly unknown outcome, hearsay, historical evidence, and multipart
  questions with only partial support.

The repair report passes its bounded regression objective. The adjacent full
simulation report remains failed because general semantic expectations are not
met. Do not report the repair's pass as a general retrieval pass.

The existing temporary workflow is `/tmp/qol-memory-confidence-simulation.mjs`;
a copy is retained as `simulate.mjs` in the baseline report directory. It builds
and freezes a worker, creates fresh stores and writes reports. Reuse this node
instead of retyping the case execution loop. Its original run-status field
describes execution; the assessment reports additionally mark semantic failures.
Before promoting it into a permanent acceptance workflow, make execution and
semantic acceptance distinct and return nonzero on unmet acceptance criteria.

### Earlier narrow shorthand proof

`reports/qol-memory/shorthand/2026-09-05T17-12-14.979Z/report.json`

The nine-fact fixture improved from 2/18 to 18/18 supported answers, with all
20 negative cases withheld. Five repetitions gave warm backend rows p95 of
0.162 ms before and 0.208 ms after. These exclude launcher debounce, IPC and
full-store loading; do not present them as end-to-end launcher latency.

This run includes isolated actual-store replay, guest launcher screenshots and
live daemon proof. It established the shorthand fix, not general understanding.

### Optional semantic verifier

`reports/qol-memory/verification/2026-09-05T16-20-28.721Z/report.json`

Development answered 62/66; the reserved corpus answered 21/24 with zero wrong
answers and 26/26 negative cases withheld. Reserved coverage of 87.5% failed
the agreed 90% gate. The historical `decision.qualified` field is authoritative;
an older execution `status: pass` did not mean qualification.

Cached answer p95 was at most 0.61 ms; verification completion p95 at most
2.20 seconds on that small fixture. These figures do not measure full-store
or launcher latency. The model occupied approximately 5.6 GB of VRAM in that
experiment; do not silently impose this resource cost on ordinary retrieval.

The reserved examples have since been inspected. They are now regression data;
use fresh unseen examples for the next qualification. A prompt probe that
recovered boot wording but accepted a retain/lose polarity error was rejected.

## What to do next

### 1. Establish an answer contract and a useful baseline

Use distinct outcomes for a supported answer, a qualified/partial answer,
ambiguous intent or conflicting evidence, and no supported answer. This is a
proposed implementation contract, not an approved visual redesign.

Confidence must concern support for this answer to this question, preserving
subject, intent, scope, polarity, conditions and relevant time/version. Keep
source reliability separate from similarity and from the ability to quote a
record. A confident report of "not tested" does not certify that a feature works.

Extend the existing simulation into a repeatable diagnostic workflow. Preserve
the baseline, add fresh subjects and unseen phrasing, and cover declarative and
legacy captures as well as explicit Q/A records. Add scope, source-lineage and
freshness fixtures; these were not exercised as metadata in the current 24 cases.

### 2. Preserve complete evidence through candidate preparation

Fix the shared evidence boundary before tuning a model. Retain the original
memory and its provenance alongside any derived question and concise answer.
Candidate retrieval and answer verification must be able to use explanations,
not only the extracted question or opening answer.

Inspect existing store, ingestion, distillation and shared retrieval owners
before designing a new persisted representation. Avoid silently rewriting the
user's raw memories. Derived indexes must invalidate when evidence, visibility,
feedback or relevant policy changes. Evaluate both explicit Q/A and useful
declarative captures.

### 3. Evaluate general answerability and agreement

Use complete candidate evidence to decide whether it supports the requested
intent and scope. Distinguish equivalent answers, additional compatible detail,
actual contradiction and different topics. Duplicate count is not independent
corroboration. Preserve conflicting evidence for the caller.

Keep deterministic matching as a fast path only where its guarantees hold.
Evaluate a general semantic stage through the existing verifier boundary; do
not assume the current provider or prompt has earned default activation.
Do not encode retail/normal or other project-specific synonyms as the solution.

### 4. Integrate and prove the default user experience

The target is useful behavior in the normal launcher flow, without the user
having to discover an experimental toggle. Decide the provider/resource policy
from measured quality and latency. If that target cannot yet be delivered,
state the limitation plainly rather than claiming an optional path solves it.

Preserve immediate retrieval, background work outside the store mutex, bounded
queues, cache invalidation, and stale-query/closed-window protection. Surface
ambiguity, conflict, unavailable verification and unsupported questions honestly.
Verify actual rendering for conditions and uncertainty; a handler verdict alone
does not prove the user sees the appropriate qualification.

### 5. Use meaningful completion gates

- Freeze new unseen cases before tuning against them. Existing inspected cases
  are regression fixtures, never a fresh held-out set.
- Require no wrong accepted answers in the agreed negative cases; measure both
  missed supported answers and inappropriate answers to ambiguous requests.
- Retain the prior 90% supported-answer coverage target for semantic promotion
  unless the contract is explicitly revised. This finite test is not a guarantee
  of universal accuracy.
- Validate qualified and partial answers separately from binary answer presence.
- Compare before/after cold, warm, background completion and visible launcher
  behavior on the same data. Include a realistic isolated store size.
- Exercise the launcher in a disposable guest, including changing queries and
  closing/reopening. Use sanitized fixtures and no host desktop automation.
- Prove the built artifact reached the running process before saying it is live.
- Give the user a short delivery statement: what improved, exact evidence,
  default enablement, performance limits, and unresolved cases.

## Progress against the next steps (2026-09-05, evening)

Landed on main from the `memory-semantic` worktree as one commit. The verifier
remains opt-in (`verify_answers` defaults to false) and needs a local Ollama
with the pinned qwen3:8b digest; the default launcher flow only gains the
outcome mapping described below.

### What landed

- Step 1, answer contract: `AskOutput` carries `outcome` (supported, qualified,
  ambiguous, conflicting, unsupported) and `reason_code` (conflicting_captures,
  below_threshold, notes_answer, capture_answer, transcript_answer,
  no_decisive_answer, verified_answer). The ask and rows payloads expose both,
  and the launcher's `parse_verdict` maps `outcome` before the legacy verdict.
  Diagnostic workflow: `node plugins/qol-memory/scripts/evaluate.mjs contract [--verify]`
  over 38 frozen cases in `tests/fixtures/answer-contract/cases.json` (the 24
  baseline cases plus legacy, declarative, scope, lineage, freshness and
  multi-record cases); the report scores the deterministic path and, with
  `--verify`, the verified path, exit 1 unless a stage qualifies.
- Step 2, complete evidence: legacy captures keep the answer prefix and the
  explanation as `recorded_answer` (the canonical answer stays the answer
  part), declarative units without an extractable question enter candidate
  preparation as facts with an empty question, the bm25 document is question
  plus recorded answer, candidates fill the prompt up to `context_byte_limit`
  (3800 to 7000 bytes), and a conflicting candidate group no longer aborts the
  snapshot. Fixture memories accept `{id, text}` so real captures replay
  through `examples/matcher-runtime`.
- Step 3, agreement through the verifier boundary: policy
  `answer-verification-v2` keeps the v1 instruction text and adds one
  `consistent` boolean; several returned IDs are accepted only when the model
  marks them consistent (smallest id wins), declarative facts take part in the
  negation and conflict guards, and `returned_facts_conflict` rejects records
  that share a question but differ in answer. A fully reworded instruction was
  tried and reverted: it answered the reserved prompt-injection query and
  flipped the Fern lose/retain polarity.

### Measured

- Contract (`reports/qol-memory/contract/2026-09-05T18-53-50.178Z/`):
  deterministic 18/28 binary matches, verified 22/28, 0 wrong answers in both.
  Baseline before this round: 10/17 on the 24 original cases.
- Frozen corpora (`node plugins/qol-memory/scripts/evaluate.mjs verify`, two
  repeats, `reports/qol-memory/verification/2026-09-05T19-58-37.583Z/`): development 63/66 answerable and reserved 23/24 in both rounds, 0 wrong answers, 26/26 negatives withheld, cached answer p95 2.5 ms, verification completion p95 2.6 s; the gate qualifies. Both corpora had been
  inspected in earlier rounds, so this is regression evidence rather than a
  fresh held-out qualification, and the 38 contract cases were inspected while
  this round was tuned; the next policy change needs a new reserved corpus.
- Real store replay (1572 captures, `reports/qol-memory/realstore-2026-09-05-run3.json`):
  "which programming language does the qol monorepo use" is answered from
  four agreeing records; "how do I stop kcd2 debug" is withheld correctly;
  "how to launch kcd2 retail" is still unanswered because qwen3:8b returns the
  two retail records but marks them inconsistent against dev records it did
  not return, and "how do I start kcd2 in retail mode" returns no IDs.
  Candidate preparation rebuilds its index per query: 125 to 145 ms warm,
  336 ms cold.
- Gate hygiene: two ollama stages run back to back race on VRAM; the second
  server saw 2.7 GiB free, offloaded the model to CPU and completion p95 rose
  from 2 s to 25 s with unchanged accuracy. Wait for the previous model to
  release before starting the next stage.

### Still broken

- Cedar `evidence-in-explanation`: a Q/A record whose answer sits in its
  explanation is withheld by the verifier.
- `vague-topic` and `unknown-platform`: the deterministic path answers where a
  clarification is due.
- `equivalent-answers-different-wording`: two records that agree in different
  words still read as a conflict on both paths.
- `exact-unit-conversion` and `explanation-negates` are withheld by the verifier.
- The real "how to launch kcd2 retail" question (above).
- Step 4 is not delivered: verification is not on by default, the launcher
  mapping is unit-tested only (no guest run), and the per-query index rebuild
  needs a cache before default activation.
- Automatic capture and coalescing are untouched.

## Operational notes

Work in the main clone on main for ordinary scoped fixes; no PR or push was
requested. Preserve concurrent work and never amend another session's commit.
Read applicable current skills rather than using stale cached paths from logs.

Useful existing checks:

```sh
cargo test -p qol-memory --lib --test answer_selection
cargo run -q -p qol -- check
qol build qol-memory
```

The latest full gate passed at
`target/qol-check/1788630978242-3439772/report.json`. The native CLI also returned
two conflicting captures and no answer for the repaired contradiction fixture.

The actual store was last inspected at
`/home/kmrh47/.local/share/qol-tray/plugins/qol-memory/units.jsonl`, approximately
49 MB. Use an isolated copy with explicit `--store` and `--no-log` for replay.
Never inject synthetic cases into the live store.

Reload through the tray's canonical single-plugin dev reload owner. The current
source route resolves to `POST /api/dev/reload/qol-memory`; rediscover the owner,
authentication and selected artifact before use. Do not print authentication
tokens or manually kill supervised daemons. No guest environments were left
running by this work.

Prior design and detailed verifier behavior:
`docs/plans/2026-09-05-qol-memory-answer-verification.md`.
