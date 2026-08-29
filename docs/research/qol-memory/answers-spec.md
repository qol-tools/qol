# qol-memory: answers are claims, not locators

Status: implementation contract, 2026-08-29. Every lane reads this whole file before editing.

## Problem

`qol-memory ask` returns `path` notes (3239 of 3769 notes, regex-harvested filename mentions with a prose tail) as its recalled rows. The answer-shaped material in the store is tiny and mostly unreachable:

- 77 pi compaction units and 136 claude compaction summaries (the latter ingested as `kind=user` and excluded from the answer pool by the boilerplate marker) hold a fixed section template with settled facts, but only 22 sessions ever got decision notes, and the JS distill runner (`decisions.mjs`, pi extension) is broken (its manifest path points at a deleted worktree) and non-deterministic (LLM).
- Assistant replies, the actual answers of every session, are never ingested (`transcript.rs` has no assistant arm).
- 20 captures exist and already answer well.

## Design

Three deterministic changes, no LLM anywhere:

1. Ingest assistant text as units of kind `assistant`; map claude compaction summaries to kind `compaction`.
2. A Rust distill (`qol-memory distill`, also run by the daemon) that turns every compaction unit into one `decision` note per claim line of its settled sections, carries forward prior `decision` notes, and drops every locator class.
3. The ask engine answers and recalls only from claim sources (decision notes, capture units, assistant units). Locator notes go to `related`. User units stay in the pool for `answer` under the existing margin rule.

## Shared contract (names every lane compiles against)

Owned by lane qm-ingest, in `plugins/qol-memory/src/ingest/mod.rs`:

```rust
pub const ASSISTANT_KIND: &str = "assistant";
pub const COMPACTION_KIND: &str = "compaction";
pub const PARSER_VERSION: u32 = 2;
pub struct IngestReport { pub files, pub appended, pub duplicates, pub reparsed, pub compactions: usize }
```

Owned by lane qm-ask, in `plugins/qol-memory/src/store/mod.rs`:

```rust
pub const CLAUDE_COMPACTION_MARKER: &str = "This session is being continued from a previous conversation";
pub const ANSWER_POOL_KINDS: [&str; 3] = ["user", "capture", "assistant"];
pub const CLAIM_UNIT_KINDS: [&str; 2] = ["capture", "assistant"];
pub const CLAIM_NOTE_CLS: &str = "decision";
pub fn in_answer_pool(kind: &str) -> bool;
pub fn is_claim_unit_kind(kind: &str) -> bool;
pub fn is_claim_note(note: &Note) -> bool;
pub fn is_compaction_unit(unit: &Unit) -> bool;
```

`is_compaction_unit` is `unit.kind == "compaction" || unit.text.starts_with(CLAUDE_COMPACTION_MARKER)`. It exists because the 136 claude summaries already in `units.jsonl` are kind `user` and can never be re-appended (keys are content hashes without the kind).

Owned by lane qm-distill, in `plugins/qol-memory/src/distill/mod.rs`:

```rust
pub struct DistillReport { pub run: Option<String>, pub unchanged: bool, pub compactions: usize, pub carried: usize, pub added: usize, pub dropped: usize }
pub fn run(store: &Store) -> anyhow::Result<DistillReport>;
```

## Lane qm-ingest

Owned paths: `plugins/qol-memory/src/ingest/mod.rs`, `plugins/qol-memory/src/ingest/transcript.rs`, `plugins/qol-memory/src/ingest/state.rs`.

1. `transcript.rs` `handle_event`:
   - claude `type == "assistant"`: content from `message.content`; text via the existing `text_of` (joins only `type == "text"` blocks, so thinking and tool_use never leak) then `redact`; skip when trimmed empty; session and cwd updated from `sessionId` and `cwd` exactly as the `user` arm does; push a unit shaped like `user_unit` but with `"kind": ASSISTANT_KIND`. Refactor `user_unit` into a kind-taking builder rather than duplicating the json block.
   - pi `type == "message"` with `message.role == "assistant"`: same text_of + redact + skip-empty, kind `ASSISTANT_KIND`.
   - claude `type == "user"` carrying `"isCompactSummary": true`: kind `COMPACTION_KIND` instead of user. Everything else about that arm stays.
2. `state.rs`: `FileState` gains `#[serde(default)] pub parser: u32`. `mod.rs`: a stored state whose `parser != PARSER_VERSION` is treated as absent (reparse from offset zero, counted in `reparsed`); every saved state carries `parser: PARSER_VERSION`. This is what makes existing transcripts yield their assistant units on the next daemon start; duplicates of already-stored user units are absorbed by the existing key set.
3. `IngestReport.compactions` counts appended units whose kind is `COMPACTION_KIND`.
4. Tests in the existing style: claude assistant text becomes an assistant unit; a claude assistant entry with only tool_use blocks yields no unit; pi assistant text becomes an assistant unit; a claude isCompactSummary entry becomes a compaction unit; a stored state with an older parser version reparses from zero; `compactions` counts.

## Lane qm-distill

Owned paths: `plugins/qol-memory/src/distill/mod.rs` (new), `plugins/qol-memory/src/distill/sections.rs` (new, pure parsing), `plugins/qol-memory/src/lib.rs`, `plugins/qol-memory/src/cli.rs`, `plugins/qol-memory/src/app/mod.rs`, `plugins/qol-memory/src/watch/mod.rs`.

### Selection

Compaction units are `store.read_units()?.items` filtered by `crate::store::is_compaction_unit`.

### Section parsing (`sections.rs`, pure functions, fully unit-tested)

Two templates, both tried on every compaction unit:

- pi template: markdown headings `## <name>`. Claim sections (case-insensitive after trimming `#`, `*`, and a trailing `:`): `key decisions`, `constraints & preferences`, `critical context`, `done`.
- claude template: numbered headings on their own line matching `^\s*\d+\.\s+(.+?):?\s*$`. Claim sections: `key technical concepts`, `errors and fixes`, `problem solving`.

Within a claim section: fenced code blocks (``` lines and everything between) are skipped; an item starts at a line whose trimmed form begins with `-`, `*`, `•`, or `\d+[.)]` followed by a space; following non-empty lines that are not a new item, heading, or fence are continuation lines joined with one space; a claim section with no items yields exactly one item from its whole text. Item text: strip the leading marker, remove `**`, collapse whitespace, trim. Keep items whose char count is between 24 and 600 inclusive, truncating longer items at 600 chars on a char boundary.

`pub fn claim_lines(text: &str) -> Vec<String>` returns the items in document order.

### Notes

For each item: `key` = first 16 hex chars of sha256 over `"decision|" + normalize(item)` where `normalize` lowercases, replaces each of the characters `` ` " ' ( ) , ; : `` with a space, collapses whitespace, and trims (this mirrors `notes.mjs` `noteKey` so carried JS notes and new notes share one key space). Note object: `{key, cls: "decision", text, source_key: unit.key, source_ts: unit.ts, source_kind: "decision-deter", session: unit.session (omitted when absent)}`.

Carry-forward: the newest existing run's notes whose `cls == "decision"` (any source_kind) are kept; every other class is dropped (`dropped` counts them). Merge carried then new, dedupe by key (first wins), sort by `(source_ts, key)` ascending.

Idempotence: when the merged key set equals the newest run's key set and the newest run held no non-decision notes, return `unchanged: true, run: Some(newest)` and write nothing.

Write: take `crate::store::lock::DistillLock::acquire(store, "distill")?`; `None` is an error `qol-memory: distill busy`. Build the run in `notes/.tmp-<name>` (files `notes.jsonl` and `report.json`), then `fs::rename` the directory to `notes/<name>` where `<name>` is the current UTC time as `YYYY-MM-DDTHH:MM:SS.mmmZ` (the same shape as existing runs; `is_run_dir_name` only checks the 11-char prefix). `report.json`: `{"name":"qol-memory notes (deterministic distill)","schemaVersion":2,"started_at","finished_at","status":"pass","inputs":{"compactions","carried"},"stats":{"added","carried","dropped"},"commands":["qol-memory distill"]}`.

### Entry points

- `cli.rs`: `distill` command (`Command::new("distill")`, about, usage `qol-memory distill [--store PATH]`, plain output `distill: run <name> added <n> carried <n> dropped <n>` or `distill: unchanged (<run>)`, `--json` prints `DistillReport`). Register it in `app_with_handlers`, add `distill_plain`/`distill_json` to `Handlers` and to the test sentinel handlers, and add it to the help and doctor never-invoke tests.
- `app/mod.rs`: after the initial ingest thread finishes its chunks, call `crate::distill::run(&store)` without holding the warm lock, then lock and `invalidate_layers()` when `unchanged == false`. Log failures with `eprintln!` and the existing `qol_runtime::probe!` pattern; a busy lock is not an error here.
- `watch/mod.rs` `drain`: when `report.compactions > 0`, run distill the same way after the ingest (release the warm lock first, re-lock to invalidate).

Tests: claim_lines on a pi-shaped summary and on a claude-shaped summary (items, continuation join, fence skip, no-items paragraph, length window); key equals the JS noteKey for a known string; run() on a temp store with one compaction unit and one prior run holding a decision note and a path note produces a run with both decision notes and no path note, and a second run() reports unchanged; the CLI tests listed above.

## Lane qm-ask

Owned paths: `plugins/qol-memory/src/ask/mod.rs`, `plugins/qol-memory/src/ask/rows.rs`, `plugins/qol-memory/src/store/mod.rs`.

1. `store/mod.rs`: the constants and predicates from the shared contract.
2. `ask/mod.rs`:
   - Fetch `NOTE_FETCH_LIMIT = 40` note hits instead of 5, partition by `is_claim_note`: the first `TOP_NOTE_LIMIT` (5) claim hits become `top_notes` (everything downstream, `note_top`, recency resolution, `out_notes`, is unchanged and now sees claims only); the first 5 locator hits become `related` as `Related { text, cls, source_ts }`, replacing the multi-intent producer at the current `related` push site (keep the `has_multi_intent` signal itself).
   - `recalled` becomes the merge of the claim `top_notes` and the first 5 `ranked_all` units with `is_claim_unit_kind`, sorted by score descending, ties by key ascending, truncated to `RECALLED_LIMIT = 8`. `Recalled` gains `#[serde(default, skip_serializing_if = "Option::is_none")] pub layer: Option<String>` holding `"note"` or `"unit"`.
   - `unit_winner` treats assistant units like user units (margin gate applies; only captures are exempt). The unit-answer reason string names the kind (`capture`, `reply` for assistant, `transcript` for user).
   - Nothing else in the verdict pipeline or the gates changes.
3. `rows.rs`: recalled unit-layer entries already resolve against the units layer; no structural change. Add a test that an assistant unit in `recalled` renders a row with kind `assistant`.
4. Tests: a path note that outranks a decision note by BM25 never appears in `recalled` or `answer` and does appear in `related`; an assistant unit appears in `recalled` with `layer: "unit"`; `recalled` is score-ordered and capped at 8; an assistant unit needs the margin like a user unit; existing tests stay green (adjust fixtures that relied on non-decision classes being recalled).

## Gate (architect runs, once per round)

`cargo test -p qol-memory`, then `env -u QOL_TRAY_HTTP_TOKEN cargo run -q -p qol -- check`.

## Acceptance

1. Gate green.
2. `qol-memory distill --json` on the live store writes a run whose notes are all `cls == "decision"`, count at least the 400 carried plus new items from the 77 pi and 136 claude compactions; a second call reports `unchanged: true`.
3. After the daemon reparses (parser version bump) `qol-memory status` shows assistant units, and `qol-memory ask "trail animation" --json` has a non-empty `recalled` where every entry is a decision note or a capture/assistant unit, `related` holds the former path notes, and no `path` cls appears in `recalled`.
4. `qol-memory rows "trail animation"` prints answer-shaped titles.

## Out of scope

The JS distill runner in qol-skills (`qol-memory-tool.ts`) stays untouched; it only ever carries decision notes forward and adds LLM decisions, which stay compatible with this run format. The JS eval harness is pinned to its own runs and is not touched.
