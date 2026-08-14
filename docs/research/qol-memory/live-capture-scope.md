# qol-memory live capture - build spec v1

Status: architect contract, grounded on the live-capture test round
(/tmp/live-capture-findings.md, flash-agent verified with real pi runs).

Goal: the store learns while the user works. A shipped pi extension appends
units to ONE append-only units.jsonl as messages and compactions happen, with
zero measurable overhead (verified 0-1ms per handler). ingest.mjs becomes the
backfill/reconcile for other harnesses and missed events. No per-turn prompt
injection; retrieval stays on-demand.

## Verified facts the build relies on

- `message_end` fires once per user message with full content + unix-ms
  timestamp; role filter cleanly excludes toolResult and assistant traffic.
- `session_compact` delivers the full CompactionEntry (summary, id, parentId,
  firstKeptEntryId, tokensBefore, reason, willRetry, ISO timestamp).
- getSessionId(), getCwd() available; getSessionFile() null in --no-session.
- Handler cost 0-1ms; no measurable wall-time delta (control 3.4-4.7s vs
  ext 2.6-3.4s in 3 runs).

## Architecture

```
pi session (any cwd)
  └─ qol-memory-tool.ts (shipped, qol-skills)
       ├─ message_end (role=user)        ──append──► store/units.jsonl
       ├─ session_compact                ──append──► store/units.jsonl
       └─ qol_memory_retrieve tool        ask.mjs --exclude-session <live session>

ingest.mjs (manual or later timer; backfill)
  ├─ snapshot.mjs (unchanged, run-based, ledger-hash future work)
  ├─ merge new snapshot units into units.jsonl (key-dedupe)
  └─ decisions.mjs (existing delta distillation)

ask.mjs (live surface)
  ├─ reads store/units.jsonl when present (fallback: newest snapshot run)
  └─ pool text-dedupe (normalized, first wins) - covers cross-path duplicates
```

The live session's own units must never answer its own prompts: the shipped
tool passes --exclude-session <getSessionId()> to ask.mjs (ask.mjs already
supports the flag from the ensemble-gate work).

## v1 scope

### 1. qol-skills: plugins/qol-project/.pi/extensions/qol-memory-tool.ts

Add handlers to the EXISTING shipped extension (it already registers
qol_memory_retrieve). All handlers must be synchronous-fast (appendFileSync).

a. Live unit append, store resolution:
   - store root: QOL_MEMORY_STORE env if set, else
     ~/.local/share/qol-tray/plugins/qol-memory (same logic as
     lib/store-path.js qolMemoryStore()).
   - kill-switch: QOL_MEMORY_LIVE_CAPTURE_DISABLE=1 skips all appends.
   - mkdirSync(store, {recursive:true}) before first append.
   - append exactly one JSON line per unit to store/units.jsonl.

b. User unit on message_end:
   - filter event.message.role === "user", non-empty text.
   - text = join of text blocks of event.message.content (same textOf()
     shape as snapshot.mjs), redacted with the SAME redact() regexes as
     snapshot.mjs (copy the function; duplication across repos accepted
     for v1, noted).
   - unit fields: key, source:"pi", file: basename(sessionFile) or null,
     session: getSessionId(), cwd: getCwd(), kind:"user",
     ts: new Date(event.message.timestamp).toISOString(), text.
   - KEY PARITY (critical): key = sha256([source, file, ts, text].join("|"))
     hex slice(0,16) - exactly snapshot.mjs unitKey() with
     source="pi", file=basename or "", ts=ISO string, text=redacted text.
     Test must prove: for the same underlying message, the live key equals
     the key snapshot.mjs would compute.

c. Compaction unit on session_compact:
   - unit fields: key (same formula), source:"pi", file, session, cwd,
     kind:"compaction", ts: new Date(entry.timestamp).toISOString(),
     text: entry.summary (redacted), filesRead: [], filesModified: []
     (CompactionEntry carries no file lists; snapshot parity is not
     required for these two fields).
   - NO distillation spawn in v1 (phase 2: detached
     decisions.mjs --session, with QOL_MEMORY_LIVE_CAPTURE_DISABLE=1 on the
     spawned process so its own prompt is not captured as a user unit).

d. Tool self-echo kill: in the qol_memory_retrieve execute(), append
   --exclude-session <getSessionId()> to the ask.mjs spawn args when a
   session id is available.

### 2. Worktree: ask.mjs live surface

a. readUnits: prefer store/units.jsonl when it exists (run label "live");
   else existing newest-snapshot-run behavior.
b. staleLayer: in live mode suppress the notes-vs-snapshot staleness path
   (notes will always trail live units until ingest runs); keep the field
   truthful with a "live units" note in the reason/output.
c. Pool text-dedupe: over user units, skip units whose normalized
   lowercase-whitespace-collapsed text was already seen (first wins, oldest
   ts first). This covers both snapshot path and live path duplicates.
   Compaction units are NOT deduped.

### 3. Worktree: ingest.mjs merge step

After the snapshot step: read the new snapshot run's snapshot.jsonl, read
store/units.jsonl keys, append missing units (key-dedupe) to units.jsonl,
bootstrap the file from the newest snapshot run if it does not exist.
Idempotent: running ingest twice adds nothing new.

### 4. qol-skills release

- Bump all 4 manifests (.pi-plugin, .claude-plugin, .codex-plugin,
  .kimi-plugin) to 0.8.16, commit atomically, push (marketplace rule;
  pi loads the live dir so no user reload needed).
- Worktree: commit locally, NEVER push.

## Test bar (all with real commands, real output)

1. Key parity: capture the same message via the ext and via snapshot.mjs on
   the same session file; assert equal keys for equal (source,file,ts,text).
2. Live capture run: QOL_MEMORY_STORE=/tmp/lct pi -p -e <shipped ext>
   "<prompt>"; assert user unit landed in /tmp/lct/units.jsonl with correct
   fields.
3. Tool-call run: user unit captured; no toolResult units.
4. Kill-switch: QOL_MEMORY_LIVE_CAPTURE_DISABLE=1 produces zero appends.
5. Self-echo: tool call log (/tmp/qol-memory-tool-calls.log) shows
   --exclude-session in the spawned ask.mjs args.
6. ingest merge: run twice, second run appends zero units.
7. ask.mjs live read: a prompt captured only in units.jsonl (not in any
   snapshot run) retrieves via ask.mjs.
8. Frozen evals unchanged: eval.mjs (units, pinned run) and skills-eval.mjs
   identical scores to before this change. Notes eval unchanged on the
   latest notes run.
9. Real-store sanity: after the work, store/units.jsonl exists, is
   valid JSONL, contains the bootstrap units.

## Out of scope v1 (phase 2 notes)

- Detached distillation on session_compact (see 1c).
- Assistant/tool-result unit kinds.
- Ledger-hash skip inside snapshot.mjs.
- Timer/cadence for ingest.mjs.
- Text-dedupe in the store file itself (read-time dedupe only).
