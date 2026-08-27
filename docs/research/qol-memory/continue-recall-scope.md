# qol-memory continue recall - build spec v1

Status: architect contract, DESIGN ONLY (no implementation). Grounded in
docs/research/qol-memory.md (continuation recall section), the current
replay-probe state (recall-new.mjs / replay.mjs), the shipped live-capture v1
(live-capture-scope.md), the store read path (lib/seal.js, lib/merge.js,
ask.mjs), the pi extension hook surface (extensions.md, qol-skills
hooks.ts / qol-memory-tool.ts), and the deleted per-turn recall hook
(git show 20b9f87^).

Goal: when a conversation is CONTINUED, surface newly-landed store context at
the session boundary, once per session, deterministic and cheap. The prime
directive of qol-memory (the memory recalls its own construction) is served
by surfacing what landed since the last session in this cwd, with the live
capture v1 prerequisites (append-only units.jsonl, sealed prefix + tail,
distill cadence) already shipped.

## Verified facts the design relies on

- Continuation hook exists in pi: `session_start` fires with
  `event.reason: "startup" | "reload" | "new" | "resume" | "fork"` and
  `event.previousSessionFile` for new/resume/fork
  (extensions.md:392-399). `before_agent_start` fires after the first prompt
  of the session with `event.systemPromptOptions.cwd` and can return
  `systemPrompt` additions (extensions.md:521-536). `session_compact` carries
  the CompactionEntry (extensions.md:476-479); `message_end` fires per user
  message (live-capture-scope.md:31-32).
- qol-memory.md:183-269 is the validated continuation-recall design:
  HOOK yes / SUMMARY mandate no, DATED VERBATIM DIFFERENTIAL BLOCK,
  once-per-session injection at the continuation boundary, cache-neutral
  because the prefix is being rebuilt (qol-memory.md:203-220). T2/T3 replay
  verdict: the cross-chat differential tier is near-empty; a surfaced block
  of size 1 resolved 0 golds; KEEP continuation-boundary injection +
  on-demand retrieve; re-run replay.mjs if the corpus ever shows genuinely
  concurrent gold-bearing sessions (qol-memory.md:254-270).
- The per-turn recall hook was MEASURED and DELETED: commit 20b9f87
  (2026-08-12) removed bin/inject-qol-memory-recall.cjs and the
  UserPromptSubmit wiring after 88.7% firing with 36.5% self-echo (146/400
  firings) and 1.8% answered+high, because the store indexed the live
  session with no exclusion. The on-demand qol_memory_retrieve tool
  replaced it. The deleted hook's shape (recovered via `git show 20b9f87^`):
  env kill-switch QOL_MEMORY_HOOK_DISABLE, MIN_LEN 8 prompt gate, RELEVANCE
  regex + distinctive.json terms, ask.mjs spawn with 4s timeout, fireLog
  appends to store/hook.log, output =
  `hookSpecificOutput.additionalContext` JSON on stdout, ALWAYS exit 0.
- hooks.ts (qol-skills plugins/qol-project/.pi/extensions/hooks.ts) already
  implements the session-boundary injection mechanism:
  SESSION_START_CONTEXT_HOOKS at hooks.ts:17, session_start stash at
  hooks.ts:130-144, before_agent_start injection guarded by session-file
  match + injectedSessionFile dedupe (once per session) at hooks.ts:148-158.
  runHook contract (hooks.ts:36-77): JSON stdout with
  `hookSpecificOutput.additionalContext`, non-zero exit = blocked (so the
  bin must always exit 0).
- The store read path: units.jsonl is append-only with a sealed prefix:
  lib/seal.js:37-50 cuts at a newline boundary, gzips the prefix
  (units.seal.gz), keeps the raw tail (SEAL_TAIL_DEFAULT 1MB, seal.js:6),
  and records prefix_len / sealed_units / created in units.seal.json.
  trySealedText (seal.js:58-76) validates the marker and concatenates
  gunzip(prefix) + raw tail. Real store today: prefix_len 15728917,
  sealed_units 3874, created 2026-08-13T13:37:26.695Z (read-only check).
  mergeUnits (lib/merge.js:10-27) unseals, rewrites, and reseals, so raw
  LINE NUMBERS are unstable across merges but unit ts are stable.
- ask.mjs exclusion machinery to mirror: BOILERPLATE_MARKERS at
  ask.mjs:30-34, isBoilerplateUnit at ask.mjs:86, EXCLUDE_SESSION at
  ask.mjs:115-116. ask.mjs is ~0.35s warm because of buildOrLoad index
  caching; the boundary read must NOT rebuild an index (buildOrLoad /
  buildIndex stay off the hot path).
- Live capture v1 is shipped (live-capture-scope.md): qol-memory-tool.ts
  appends user units on message_end and compaction units on
  session_compact, redacted with the same regexes as snapshot.mjs, key
  parity via unitKey, kill-switch QOL_MEMORY_LIVE_CAPTURE_DISABLE=1, and
  --exclude-session on the tool call. ctx.sessionManager.getCwd() /
  getSessionId() / getSessionFile() are available to handlers.
- qol-memory.md hard rule: "Never inject memory into the prompt" applies to
  PER-TURN injection; once-per-session at the boundary is the documented
  exception (qol-memory.md:180-182, 218-220).

## Architecture

```
pi session (any cwd)
  └─ session_start (reason startup|resume|new|fork)
       └─ hooks.ts SESSION_START_CONTEXT_HOOKS
            └─ bin/inject-qol-memory-continue.cjs   (NEW, mirrors deleted
                 │                                    inject-qol-memory-recall.cjs)
                 │  stdin: { cwd, session_id, session_file, reason }
                 │  1. read marker <store>/continue.marker.json (per-cwd ts)
                 │  2. read units.jsonl (sealed tail fast path, full fallback)
                 │  3. delta = units ts > marker.ts, session != current,
                 │     boilerplate/short excluded, per-session caps, newest first
                 │  4. gate: delta >= MIN_DELTA -> emit compact block
                 │     else emit nothing (abstain), log to store/hook.log
                 │  5. write marker (atomic tmp+rename) AFTER successful read,
                 │     always (gate or not)
                 └─ stdout: hookSpecificOutput.additionalContext | nothing
       └─ stashed context injected once at first before_agent_start
            (existing hooks.ts:148-158 mechanism, cache-neutral prefix rebuild)

qol_memory_retrieve tool (unchanged): the on-demand surface for anything
the boundary did not surface. --exclude-session stays.
```

## Design decisions

### A. Boundary: what is a continuation?

Recommendation: the boundary is `session_start` in the existing hooks.ts
SESSION_START_CONTEXT_HOOKS slot, for ALL reasons (startup, resume, fork,
new). No idle timer in v1.

Rationale:

- `session_start` with reason "startup" is the common real continuation
  (pi opens in a cwd and continueRecent picks up the most recent session);
  "resume" is the explicit /resume case (qol-memory.md:203 names exactly
  this: "pi session_start reason=resume"). Fork is a derived continuation.
  Reason "new" also fires the hook: a fresh session in the same cwd has
  empty context, so the delta since the marker is most valuable there.
  Reason is carried as metadata, not a filter, in v1.
- Metadata available at the hook (hooks.ts:131-144): cwd via
  ctx.sessionManager.getCwd(), session id via getSessionId(), session file,
  and event.reason. The existing stash-and-inject guard
  (hooks.ts:148-158) guarantees once-per-session delivery at the first
  before_agent_start, when the prefix is being built anyway (cache-neutral,
  qol-memory.md:218-220).
- An idle-based boundary (first prompt after N hours) does NOT fire a pi
  event inside a live session, so it would need a timer or a check on every
  before_agent_start, which reopens the per-prompt cost we deleted.
  Deferred; the next session_start covers the same information, and the
  marker makes the idle duration implicit in the delta's ts range.
- No message_start / per-turn path: per-turn injection was measured and
  deleted (commit 20b9f87); the boundary is the only prompt-text surface
  for v1.

### B. Delta semantics: what is "newly-landed context"?

Recommendation: newly-landed = units in the store with
`ts > marker.ts(cwd)` AND `session != current session id`, kind in
{user, compaction}, text >= 40 chars after trim, not boilerplate-marked.

Rationale:

- ts is the delta key because unit ts survive merges; line numbers do not
  (lib/merge.js rewrites and reseals). Marker ts is per-cwd, giving each
  workspace its own sliding window.
- Self-echo exclusion: the current session id is ALWAYS excluded, even when
  its units have ts > marker. This is the structural fix for the measured
  failure (commit 20b9f87: 146/400 firings were the hook reading the live
  session's own units back at it). For resume/startup the current session
  id equals the continued session, so its own transcript is never re-surfaced;
  for a new session the previous session's units legitimately ARE the delta
  (they landed while the user was away and are not in the new transcript).
  No marker-session exclusion is needed: the session that wrote the marker
  is exactly the one being continued, so the current-session rule covers it.
- Exclusions mirror ask.mjs: BOILERPLATE_MARKERS verbatim
  (ask.mjs:30-34), redaction already applied at capture time
  (qol-memory-tool.ts redact()), short/empty text dropped.
- Compaction units (session summaries) are included: they are exactly "what
  happened while you were away", capped at 1 per session in the block.
- Marker advances on every successful read regardless of the gate, so a
  below-threshold delta is not re-surfaced forever.

### C. Marker layout: where the last-seen lives

Recommendation: single file `<store>/continue.marker.json`, schema
`qol-memory-continue-v1`, holding a map keyed by cwd:

```json
{
  "schema": "qol-memory-continue-v1",
  "cwds": {
    "/path/to/project": {
      "ts": "2026-08-13T18:02:41.123Z",
      "session": "019f…",
      "units_count": 3892,
      "updated": "2026-08-13T18:02:41.123Z"
    }
  }
}
```

Rationale:

- Lives beside units.seal.json / manifest.json / hook.log in the store root;
  store root comes from lib/store-path.js qolMemoryStore() (QOL_MEMORY_STORE
  env first), the same resolution qol-memory-tool.ts uses.
- Per-cwd key: the vision is "new session in same cwd/project"; cwd is
  stable across sessions while session ids are not, and startup continuations
  carry no previousSessionFile to key on.
- `ts` = wall-clock read completion time (host clock; unit ts come from pi
  message timestamps on the same host clock family, so the comparison is
  homogeneous). `units_count` = total line count of units.jsonl at read
  time, used ONLY as a reset/rotation detector (if the current line count is
  below the marker's count, the store was rebuilt: treat delta as empty and
  refresh). It is never used for delta computation because merge reseals
  invalidate line positions (lib/merge.js:10-27).
- Writer: the injector bin, only, and only AFTER a successful read
  (crash safety). Write via the seal.js writeTmpRename pattern
  (seal.js:31-35): tmp file + atomic rename. Read-modify-write preserves
  other cwds' keys; a crash between read and write leaves the old marker,
  so the next continuation re-reads the same delta at most once more (a
  benign duplicate because the block carries facts with provenance, never
  answers). Store-reset case: rewrite the marker (old counts invalid).
- If the read fails (missing/corrupt units.jsonl), the marker is untouched
  and the hook abstains (see E), logging the reason.

### D. Ranking + format

Recommendation: newest-first by ts desc, one pass over the delta set, no
index, no BM25 in v1. Per-session caps: at most 2 user units + 1 compaction
unit per session. Total block size k = 3. Exclusions as in B.

Rationale:

- The replay verdict says the tier is near-empty (qol-memory.md:254-268), so
  ranking quality is moot for v1; the on-demand tool owns relevance. A
  JITIR-style relevance gate against the first prompt is a later refinement
  (see non-goals) and would need a prompt read at before_agent_start plus a
  small BM25 pass, compute v1 deliberately does not spend.
- Session caps stop one chat log from flooding the block; newest-first is
  the honest ordering for "what landed since I was here".

Exact block format (deterministic, verbatim snippets, provenance per line):

```
[qol-memory continue] N unit(s) landed in the store since your last session here (2026-08-13T13:37:26Z):
  NEW 2026-08-13T17:55:12.000Z user  019f…234  "snippet text, whitespace-collapsed, <=140 chars"
  NEW 2026-08-13T18:02:41.000Z compaction 019f…9ab  "snippet text…"
```

- Header: count + marker ts of the previous session (the "since" anchor).
- Each line: `NEW <ISO ts> <kind> <session-id-8> <key-8> "<snippet>"`.
  Snippet = first 140 chars of the redacted text with whitespace collapsed
  (the recall-new.mjs:120 snippet precedent; the tool surface caps at
  240/280, qol-memory-tool.ts).
- No verdict, no confidence, no "Anchor your reply" directive: the old
  hook's directive line is dropped in v1. The block is data, not an answer
  (see E).
- Determinism: same store + marker + inputs => byte-identical block.

### E. Gate: when to show nothing

Recommendation: fire only when, after all exclusions (B), the delta has
>= MIN_DELTA = 2 units. Otherwise abstain: the hook emits nothing.
Additionally silent when: the store or units.jsonl is absent; the read
fails; QOL_MEMORY_CONTINUE_DISABLE=1; flag file `<store>/continue.disabled`
exists. Every quiet decision is appended to `<store>/hook.log`
(fireLog pattern from the deleted hook).

Rationale:

- MIN_DELTA = 2 is grounded in the replay measurement: a surfaced block of
  size 1 resolved 0 golds (qol-memory.md:259-261); a lone unit at the
  boundary is indistinguishable from noise, and the on-demand tool still
  finds it. "Changed but not relevant => silence" is the field-supported
  operating point (qol-memory.md:196-199).
- "Never wrong" doctrine: the block surfaces verbatim facts with provenance
  (ts, session, key) and NEVER answers, verdicts, or interpretations. The
  model may ignore it; there is no directive language. Abstain means zero
  bytes, not a placeholder.
- Honest and traceable silence (qol-memory.md:246-249): the log distinguishes
  "quiet because nothing landed" from "quiet because disabled" from "quiet
  because read failed", so silent degradation is observable.
- Kill-switch hierarchy: env QOL_MEMORY_CONTINUE_DISABLE=1 (mirrors
  QOL_MEMORY_HOOK_DISABLE from the deleted hook and
  QOL_MEMORY_LIVE_CAPTURE_DISABLE in qol-memory-tool.ts) hard-kills before
  any read; the flag file gives a persistent per-machine mute without env
  plumbing. When disabled, the marker is NOT advanced.

### F. Cost budget

Recommendation: worst-case injected block <= ~620 bytes (~160 tokens);
read path tail-only in the common case; no index build, no LLM anywhere on
the hot path; one-shot per session start.

Rationale and mechanics:

- Read: readFileSync(units.jsonl) raw. If the continue marker exists and
  `marker.ts >= seal.created` (units.seal.json), parse ONLY the unsealed
  tail `raw.subarray(prefix_len)` (<= SEAL_TAIL_DEFAULT = 1MB, seal.js:6),
  because every unit in the sealed prefix predates the marker by
  construction. If the marker predates the seal (an ingest merge ran while
  the user was away), fall back to the full trySealedText path
  (seal.js:58-76); real store worst case ~4MB gzip + ~3.9k lines today,
  a one-shot ~50-150ms. No indexcache, no buildOrLoad, no buildIndex:
  newest-first needs none of them.
- Compute: O(tail lines) filter + O(k) pick; no scoring.
- Injection: once per session at before_agent_start, attached to the prefix
  being rebuilt (cache-neutral, qol-memory.md:218-220). No per-turn cost;
  the hooks.ts injectedSessionFile guard (hooks.ts:152-157) prevents
  re-injection.
- Wall budget: <5ms typical, <150ms worst, comfortably inside the 5s
  runHook timeout (hooks.ts:47). The 10k harness ceiling (qol-memory.md:220)
  is not approached (~160 tokens at k=3).
- Determinism: same store + marker => same delta => same block; no clock
  sampling except the marker ts recorded at read completion.

### G. Test plan

test-continue.mjs, the established sandbox pattern (test-recency.mjs:
tmpdir store via QOL_MEMORY_STORE, check() pass/fail lines, non-zero exit
on failure). Pure delta logic lives in lib/continue.js so tests run without
spawning. Cases:

1. Delta: marker ts X => exactly the later units surface, newest first.
2. Self-echo: units with the current session id excluded even when ts >
   marker (the 146/400 regression guard).
3. Boilerplate: unit containing "[qol session bridge]" (ask.mjs:30-34)
   excluded.
4. Session caps: 5 user units from one session => at most 2 user + 1
   compaction in the block.
5. Gate: delta of 1 => empty output; delta of 2 => block fires.
6. Disabled: env var and flag file each => empty output, marker NOT
   advanced, log line written.
7. Crash safety: corrupt units.jsonl => empty output, marker untouched,
   exit 0, log line written.
8. Marker write: after a successful run, marker ts = read completion,
   units_count = line count, other cwds' keys preserved (read-modify-write).
9. Store reset: marker units_count > current line count => empty delta +
   marker refreshed.
10. Seal interplay: with a units.seal.json present, marker.ts >= seal.created
    => tail-only path; marker.ts < seal.created => full path; both paths
    surface the same key set.
11. Determinism: two runs on identical inputs => byte-identical stdout.
12. Exit contract: always exit 0; stdout is either the
    hookSpecificOutput.additionalContext JSON or empty.
13. Format: exact block shape, snippet truncation at 140, whitespace
    collapse, provenance fields present on every line.

Tests never touch the real store (~/.local/share/qol-tray/plugins/
qol-memory/); all sandboxes live in tmpdir. Optionally mirror the
live-capture test bar: one end-to-end run with QOL_MEMORY_STORE=/tmp/...
pi -p -e <ext> verifying the block appears exactly once at the first
before_agent_start and the marker file lands in the sandbox store.

### H. Integration point

Recommendation: qol-skills plugins/qol-project/.pi/extensions/hooks.ts,
in the existing SESSION_START_CONTEXT_HOOKS list (hooks.ts:17-19) and the
existing session_start handler (hooks.ts:130-144) that stashes context and
injects at before_agent_start (hooks.ts:148-158). New bin:
plugins/qol-project/bin/inject-qol-memory-continue.cjs.

Rationale:

- The mechanism already exists and is proven: session_start fires, the
  stashed context lands once at the first before_agent_start of the
  session, deduped by session file. The continuation block is exactly the
  kind of SessionStart context inject-qol-cli-context.cjs already emits
  (same hookSpecificOutput.additionalContext contract, same always-exit-0
  discipline enforced by runHook at hooks.ts:36-77).
- ONE required change to hooks.ts: the session_start payload at
  hooks.ts:137 currently passes only session_id; extend it to
  { session_id, cwd, session_file, reason } (ctx.sessionManager.getCwd()
  and getSessionFile() are available, as qol-memory-tool.ts uses them).
- The bin mirrors the deleted inject-qol-memory-recall.cjs shape: stdin
  JSON, fireLog to store/hook.log, bail() = exit 0 with no stdout,
  ok() = exit 0 with the JSON block. Read/modify/write the marker per C,
  compute the delta per B, gate per E, format per D.
- Silencing: QOL_MEMORY_CONTINUE_DISABLE=1 (env) plus flag file
  <store>/continue.disabled, both short-circuiting before the read.
- No changes to qol-memory-tool.ts in v1 (the tool call and its
  --exclude-session stay as the on-demand surface).
- Release mechanics per live-capture-scope.md section 4: bump all 4 qol-skills
  manifests, commit atomically, push (marketplace rule; pi loads the live
  dir). Worktree commits stay local, never pushed.

### I. Non-goals for v1

- No qol-tray plugin daemon / watcher / tier-2 differential: replay.mjs
  measured the cross-chat tier as near-empty and the watcher as not worth
  building now (qol-memory.md:254-268). Re-run replay.mjs if the corpus
  shows genuinely concurrent gold-bearing sessions before revisiting.
- No per-prompt retrieval: deleted in commit 20b9f87; the on-demand
  qol_memory_retrieve tool remains the per-turn surface.
- No LLM summarization or distillation at the boundary: the block is
  verbatim; decisions.mjs keeps its own cadence
  (qol-memory-tool.ts session_compact handler).
- No cross-device sync: marker and delta are single-host by design;
  profile-sync is a later concern (qol-memory.md machinery list).
- No relevance ranking against the first prompt (JITIR step 2): newest-first
  only until the tier fills.
- No idle-timer boundary, no message_start surface, no snapshot-run
  fallback when units.jsonl is absent (snapshot fallback would need the
  expensive index path for no live benefit).

## Open questions

- k = 3 and MIN_DELTA = 2 are design guesses grounded in the replay
  measurement, not field-calibrated. A field round like the live-capture
  test round should measure firing rate and block size on real sessions
  before shipping the qol-skills bump; the hook.log is the measurement
  instrument.
- Compaction units in the block (recommended, capped at 1/session) versus
  user-only: the field round should confirm they read as useful context
  rather than noise.
- Marker keyed by raw cwd string: sessions in shared or relocated
  directories (e.g. /tmp, renamed worktrees) fragment the marker map.
  Acceptable for a personal single-host store; worth noting if worktrees
  churn.
- Clock homogeneity: marker ts and unit ts both come from the host clock,
  but a backward system-clock jump makes the marker ts lie in the future
  and silences the boundary until the clock catches up. The units_count
  reset check does not cover this; accepted as a rare, self-healing
  limitation.
- The hooks.ts payload extension (H) is a qol-skills change; the worktree
  owns only lib/continue.js + bin + tests in v1, so the first real
  integration crosses repos on the qol-skills side.
- hooks.ts fires for reason "new" too (A); if field use shows /new sessions
  read the block as noise, restricting to startup/resume/fork is a one-line
  filter, but the recommendation stands: a fresh session has the least
  context and the most to gain.
