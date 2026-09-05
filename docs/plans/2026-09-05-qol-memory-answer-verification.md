# Memory answer verification

The launcher retrieves useful memories but misses paraphrased questions. The
comparison workflow owns the prior evidence. Its answer-aware development probe
recovered the answerable cases while accepting changed negation and a different
numeric identifier. Those results do not qualify the model for promotion.

Implement one verifier boundary shared by the experiment and runtime. Model
output chooses an existing fact; it never supplies a replacement answer.
Deterministic checks reject unknown keys, changed negation, unsupported numeric
or symbolic identifiers, and contradictory records. Conservatively rejecting
changed negation also withholds legitimate paraphrases; measure those misses.

Freeze policy, prompt, model identity, and fresh fixture hashes before a new
held-out run. Preserve the previous held-out corpus as historical evidence.
Require zero wrong accepted answers and at least 90% answer coverage on the new
corpus before runtime promotion. Report unsupported cases separately from
malformed model output and provider failures.

The accepted interaction design returns retrieval results immediately and
verifies unfamiliar questions in the background. Measure initial response and
cached answers against 200 ms; report verification completion separately with a
10 second warm p95 target. This replaces the previous synchronous model latency
gate because inference no longer occupies the user input path. Cold model load
and resource contention remain separate measurements.

Runtime jobs must not hold the memory-store mutex during inference. Bound queued
work and keep workers asleep when idle. Launcher refreshes must stop when the
query changes or the surface closes; a late result cannot replace a newer query.
Persist a binding to the exact question, caller visibility, evidence revision,
and verifier policy/model revision. New conflicting evidence, edits, deleted
facts, visibility changes, or negative feedback invalidate the binding. Keep
explicit confirmations distinct from model predictions.

Trace decision: enrich the existing memory daemon probe with queued, completed,
cached, and unavailable events containing hashes and decision reason codes.
Evidence invalidation changes the binding identity. Never include question or
memory text in runtime probes. Verify the daemon contract with fixture stores
and the launcher behavior in a disposable guest.

## Implemented boundary

The daemon keeps deterministic retrieval first. If it has no answer and no known
capture conflict, it prepares up to eight visible candidates by lexical rank,
as many as fit the prompt byte budget in `profile.json`, and queues local
verification. Candidates carry the complete recorded evidence: explicit Q/A
records, legacy captures with their explanation, and declarative captures
without an extractable question. The worker returns stored IDs only; the
original capture supplies the answer and provenance. Model agreement in both
candidate orders, meaning checks, identifier checks and conflict checks are
required, and several returned IDs are accepted only when the model marks them
consistent (policy `answer-verification-v2`). Accepted answers have medium
confidence.

One sleeping worker owns inference, with four queued requests and 256 derived
bindings. A newer launcher query replaces queued work for that launcher lane.
Bindings include the exact query, visible evidence revision, caller, excluded
session, model digest and policy digest. A restarted daemon can load them
without invoking the provider. The warm store verifies the old file prefix
before treating a larger file as an append, preventing rewritten evidence from
leaving stale answers in memory. First capture now creates a missing store.

The existing ask and rows protocol adds `verification.status`, plus `outcome`
(supported, qualified, ambiguous, conflicting, unsupported) and `reason_code`,
which the launcher maps before the verdict string. Agents repeat an ask when
verification is pending. The launcher displays related memories immediately,
then refreshes every 500 ms while that query remains visible, for at most 60
seconds. Query generations and flow epochs discard stale responses after
typing, leaving the flow, closing, or reopening the launcher.

`Understand question variations (experimental)` is disabled by default. It
requires Ollama and the exact local `qwen3:8b` digest in
`plugins/qol-memory/src/verification/profile.json`. Run `ollama pull qwen3:8b`
before enabling it and restarting Memory. An empty endpoint starts an owned
loopback provider on demand; an explicit endpoint must be a literal loopback
HTTP address. Runtime never downloads a model. Missing or incompatible models,
oversized evidence and malformed responses produce an unavailable state while
ordinary retrieval remains usable. In-process CLI calls with an explicit
`--store` retain deterministic behavior; semantic verification belongs to the
long-lived daemon.

## Measured result and limits

Run the production-path comparison with:

```sh
node plugins/qol-memory/scripts/evaluate.mjs verify --prepare
```

It freezes model, policy and fixtures; builds and hashes copied Rust workers;
uses fresh fixture stores for each repeat; drives the actual daemon request
handler; and measures immediate rows, background completion and cached ask.
It evaluates the reserved corpus only after development passes. Every repeat
must have zero wrong answers, at least 90% answer coverage, response p95 below
200 ms and completion p95 below 10 seconds. Failed qualification returns a
nonzero exit code. Provider failures are failures rather than abstentions.

The retained run at
`reports/qol-memory/verification/2026-09-05T16-20-28.721Z/report.json`
evaluated 127 development questions and 50 reserved questions, twice each.
Development answered 62/66 with no wrong answers. The reserved set improved
from 1/24 to 21/24 answerable questions, with all 26 negative cases correctly
withheld in both rounds. Cached answer p95 was at most 0.61 ms and completion
p95 at most 2.20 seconds. **The 87.5% reserved coverage fails qualification.**
The historical report's `decision.qualified` is authoritative; its old `status`
field recorded successful workflow execution even for a rejected candidate.
The workflow now reports that rejection as failed.

Later on 2026-09-05 the v2 policy qualified the same workflow at two repeats:
`reports/qol-memory/verification/2026-09-05T19-58-37.583Z/report.json`,
development 63/66 and reserved 23/24 with no wrong answers, 26/26 negatives
withheld, cached answer p95 2.5 ms and completion p95 2.6 s. Both corpora had
been inspected by then, so this is regression evidence, not a fresh held-out
qualification. The additional frozen cases live in
`tests/fixtures/answer-contract/cases.json` (`evaluate.mjs contract`) and were
inspected during that round too; the next policy change needs a new reserved
corpus. Details and remaining failures:
`docs/plans/2026-09-05-qol-memory-semantic-retrieval-handoff.md`.

The inspected corpus was then retained as `heldout-third.json` and promoted
into `development.json` (40 facts, 177 cases), and a fourth reserved corpus
was written blind with 16 fictional facts in explicit Q/A, legacy explanation
and declarative shapes plus 52 cases (`{id, text}` facts are accepted by the
scorer since then). On the merged development store the model obeyed the
injection query that names a memory id, so `check()` now rejects any query
that names a candidate id as a whole word (`instruction_in_query`; only ids
containing a digit, hyphen or underscore count). With that guard development
answers 84/90 with no wrong answers, but the fourth corpus does not qualify
(`reports/qol-memory/verification/2026-09-05T21-07-31.760Z/report.json`):
24/26 answerable with 2 wrong answers in both rounds, completion p95 2.5 s.
The model answered a yes/no question whose verb reverses the recorded
polarity (drop versus keep) and reused a record whose stored answer is
explicitly uncertain; it also withheld both retail-mode questions about a
two-mode tool, the same shape as the user's real launch question. That corpus
is now inspected and becomes development evidence at the next promotion; the
next policy change needs a fifth blind corpus.

The reserved misses were `Cobalt debug startup command`,
`fire up Cobalt with debugging enabled`, and `how do I shut down Cobalt`.
At that checkpoint, development missed `how to boot kcd2 debug`.
A subsequent development-only prompt probe recovered `boot` but regressed
`does Fern lose offline edits after restarting` to the stored positive answer
about retaining edits. That prompt was rejected and is not the shipped policy.
Its evidence is `/tmp/qol-memory-scope-probe/report.json`.

These are agent-authored fixtures, not an independent human evaluation or a
guarantee for arbitrary phrasing. The former reserved corpora are retained as
`heldout-first.json` and `heldout-second.json`; both were promoted into
development after their failures were inspected. The current `heldout.json`
has now been inspected too and must become development evidence before another
policy is qualified against a new reserved corpus. Model selection must improve
meaning discrimination without weakening the negative cases. Automatic capture
triggers and semantic coalescing remain subsequent work.

## Runtime verification

The disposable Linux Mint Cinnamon run
`linux-mint-cinnamon-18d2772de4c1f360-243f7f-1` exercised the real launcher and
daemon with a controlled delayed local provider. Screenshots under
`target/qol-env/cases/<run-id>/` show pending retrieval
(`screenshot-1788625392800.png`), the completed KCD2 answer
(`screenshot-1788625805774.png`), switching away from a pending KCD2 request to
the QOL language answer (`screenshot-1788625923674.png`), and remaining closed
after a pending verification completes (`screenshot-1788626105491.png`).
This establishes the asynchronous UI behavior, not model accuracy. The guest
used the earlier provider profile and response schema; the production-path
headless evaluation used the final pinned profile. The guest was shut down and
`qol env runs` confirmed that no development environments remained running.

Regression tests cover queue replacement and limits, duplicate jobs, persisted
binding reuse without inference, missing-store capture, edits, deletion,
conflicting evidence, negative feedback, caller visibility, session exclusion,
and changing evidence while inference is running. Launcher tests reject old
query generations and prior flow epochs.

## Default shorthand retrieval

The deterministic selector now accepts shorthand containing at least two
distinct content terms, such as `kcd2 debug` and `qol monorepo language`.
Every query term must match a recorded question, and all matching answers must
agree. Negation, project identifiers, symbolic names and command direction
remain significant. A single topic or conflicting modes still yields choices.
The existing launch vocabulary also includes `boot`. This path needs neither
the experimental setting nor a model and returns the existing launcher answer
card immediately.

Recorded alternatives such as `./tool dev (or -d)` agree with their explicitly
written commands. Other annotations, different arguments and differently cased
paths remain distinct. This resolves the false conflict between three captures
in the user's actual store without changing those memories.

The shorthand report at
`reports/qol-memory/shorthand/2026-09-05T17-12-14.979Z/report.json` compares frozen
before and after workers on 38 questions with five warm repetitions each.
Correct answers increased from 2/18 to 18/18; all 20 negative cases remained
unanswered. Warm rows p95 changed from 0.162 ms to 0.208 ms on the nine-fact
fixture. These timings exclude launcher debounce, IPC and full-store loading.
An isolated copy of the user's store returned the same capture for `kcd2 debug`,
`how to run kcd2 debug` and `how to boot kcd2 debug`, also answered the language
shorthand, and withheld the launch answer for `stop kcd2 debug`.

The existing `QOL_MEMORY_DAEMON` probe now records rows verdict, match count,
conflict count and verification state without query or memory text. Integration
tests exercise both CLI selection and daemon rows with model verification off,
including noisy stores, conflicting questions, command aliases, direction,
negation, C++/C# identity and Unicode names.
