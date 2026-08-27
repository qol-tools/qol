# Phase 2b spec: qol-memory capture as an agent tool

Status: architect contract for the `qm-capture-write` and `qm-capture-recall` lanes. Source plan: `docs/research/qol-memory/interface-plan.md` (section 4, `[action.capture]`). Phase 2 (resident daemon, 2e1a8be1d) left `capture` as a whole-unit action that only the CLI and the pi extension can call. This phase makes it an agent tool: an agent passes one settled fact plus its project directory, the daemon builds the unit, and the fact comes back through `ask` and the session-start continue block with honest provenance (`kind = "capture"`, `source = "agent"`), never disguised as the user's own words.

Rules for every lane: edit only the paths in your ownership row; never run build, test, lint, format or git commands; add no code comments; never use the em-dash character anywhere. Every public signature below is exact; do not rename, reorder or widen it. Report changed files and lines plus conscious deviations, nothing else.

## 1. Goal

- `tools/list` on `POST /api/mcp` lists `qol-memory__capture` next to `qol-memory__ask` and `qol-memory__status`, with input `{ text, cwd }`.
- `tools/call qol-memory__capture {"text": "...", "cwd": "/abs/project"}` appends exactly one unit and answers `{ "appended": 1, "key": "<16 hex>" }`; the same call again answers `{ "appended": 0, "key": "<same>" }`.
- `qol-memory capture --text '<fact>' --cwd PATH [--store PATH]` does the same from a terminal; `--unit '<json>'` keeps working unchanged.
- A captured fact is answerable by `ask` (layer `unit`, `source_kind` `capture`, reason `units layer answer (agent capture), confidence capped medium`) and is injected by `continue` like a user unit.
- Stores without capture units produce byte-identical `ask` output before and after this phase (parity 73/73 against `ask.mjs` on the fixture store).

## 2. Ownership

| Lane | Owned paths |
|---|---|
| `qm-capture-write` | `plugins/qol-memory/plugin.toml`, `plugins/qol-memory/qol-runtime.toml`, `plugins/qol-memory/src/cli.rs`, `plugins/qol-memory/src/app/request.rs`, `plugins/qol-memory/src/ingest/mod.rs` |
| `qm-capture-recall` | `plugins/qol-memory/src/store/mod.rs`, `plugins/qol-memory/src/ask/mod.rs`, `plugins/qol-memory/src/app/warm.rs`, `plugins/qol-memory/src/continue_recall/mod.rs`, `docs/research/qol-memory/ask.mjs` |

Two lanes never share a file. `qm-capture-write` may call `crate::store::in_answer_pool` and `qm-capture-recall` may call `crate::ingest::capture_unit`, `crate::ingest::CAPTURE_KIND`; both exist once both lanes land and the architect runs the gate after the fan-in.

## 3. Lane `qm-capture-write`

### 3.1 `plugins/qol-memory/plugin.toml`

Add directly after the `[action.status]` block (the host only executes catalogued actions, `validate_catalog_action_membership`):

```toml
[action.capture]
label = "Capture"
args = ["capture"]
```

### 3.2 `plugins/qol-memory/qol-runtime.toml`

Replace the `[action.capture]` block with:

```toml
[action.capture]
description = "Append one settled fact to the store"
agent_tool = true
tool_description = "Remember one settled fact for future sessions. Write one self-contained sentence carrying the identifiers a later reader needs (paths, commits, names, dates). It is stored verbatim, scoped to cwd, and comes back through qol-memory__ask and the session-start continue block. Calling it twice with the same text and cwd stores one fact."
input = { text = "the fact as one self-contained sentence", cwd = "absolute working directory of the project the fact belongs to" }
```

`qol_mcp::input_schema` marks every input required; that is intended for both fields.

### 3.3 `plugins/qol-memory/src/ingest/mod.rs`

Add next to `unit_key`:

```rust
pub const CAPTURE_SOURCE: &str = "agent";
pub const CAPTURE_KIND: &str = "capture";

pub fn capture_unit(text: &str, cwd: &str, ts: &str) -> Value
```

Behavior: `key = unit_key(CAPTURE_SOURCE, cwd, None, text)` so the key depends on `cwd` and `text` only (idempotent remember; the second identical capture appends 0). The object is exactly `{ "key", "source": "agent", "cwd", "kind": "capture", "ts", "text" }` with no `file` and no `session` member. Callers pass already-trimmed, non-empty `text` and `cwd`; `capture_unit` does no validation.

Tests (in the existing `tests` module): `capture_unit_key_ignores_ts_and_depends_on_cwd_and_text` (same text and cwd with two different ts values give one key; a different cwd gives another key; the object carries source `agent`, kind `capture`, the ts passed in, and neither `file` nor `session`).

### 3.4 `plugins/qol-memory/src/app/request.rs`

`capture` accepts two input shapes:

- `input.unit` present: must be an object (existing behavior and existing error text).
- `input.unit` absent: `text = string_field(input, "text", "capture")?` and `cwd = string_field(input, "cwd", "capture")?`; trim both; empty `text` fails with `capture: input.text must not be empty`, empty `cwd` fails with `capture: input.cwd must not be empty`; build the unit with `crate::ingest::capture_unit(text, cwd, &crate::text::now_iso())`.

Both shapes then run the existing append and warm push unchanged and answer `json!({ "appended": appended, "key": <the unit's "key"> })`. The key is the unit's own `key` string, taken from the object in both shapes.

Tests: `request_capture_from_text_is_idempotent_and_recallable` using the existing `warm_state` and `respond` helpers: first call `{"text": "...", "cwd": "/tmp/proj"}` answers appended 1 and a 16 hex key equal to `capture_unit(..)["key"]`; the same call answers appended 0 and the same key; a following `ask` request with a query built from distinctive words of the text returns `answer.layer == "unit"` and `answer.source_kind == "capture"` (this relies on the recall lane's predicate landing in the same round). Add `request_capture_rejects_empty_text` covering whitespace-only text (error text above).

### 3.5 `plugins/qol-memory/src/cli.rs`

- `USAGE_CAPTURE` becomes `usage: qol-memory capture (--unit '<json>' | --text '<fact>' --cwd PATH) [--store PATH]`.
- `capture_command` `.about("Append one settled fact or one whole unit to the memory store.")`, `.detail("Pass a fact with --text and --cwd, or a whole unit as a JSON object with --unit.")`; output and exit lines unchanged.
- `parse_capture_invocation` gains `--text <value>` and `--cwd <value>` (use `value_flag_with` like `--store`). Rules, each a usage error (exit 64) with the exact message: `--unit` together with `--text` or `--cwd` -> `--unit cannot be combined with --text or --cwd`; `--text` without `--cwd` -> `--text requires --cwd`; `--cwd` without `--text` -> `--cwd requires --text`; whitespace-only `--text` -> `--text must not be empty`; whitespace-only `--cwd` -> `--cwd must not be empty`. With `--text` and `--cwd` the invocation's `unit` is `crate::ingest::capture_unit(text.trim(), cwd.trim(), &crate::text::now_iso())`. `CaptureInvocation` keeps its two fields; the daemon request stays `{ "unit": ... }` for both shapes, so the socket path and the in-process path are unchanged.
- Plain output stays `appended: <n>`; JSON output stays `{ "appended": n }`.

Tests: extend `capture_requires_a_json_object` only if its fixtures now conflict; add `capture_text_flags_validate` (the five usage errors above) and `capture_text_builds_a_capture_unit` (parse `["capture", "--text", " fact ", "--cwd", "/p"]` and assert `unit["kind"] == "capture"`, `unit["cwd"] == "/p"`, `unit["text"] == "fact"`, `unit["key"] == capture_unit("fact", "/p", "x")["key"]`). Keep the existing help tests passing; `help capture` must still list the command.

## 4. Lane `qm-capture-recall`

### 4.1 `plugins/qol-memory/src/store/mod.rs`

Add next to `BOILERPLATE_MARKERS`:

```rust
pub const ANSWER_POOL_KINDS: [&str; 2] = ["user", "capture"];

pub fn in_answer_pool(kind: &str) -> bool
```

`in_answer_pool` is true exactly for the two listed kinds. Test: `answer_pool_accepts_user_and_capture_only`.

### 4.2 `plugins/qol-memory/src/ask/mod.rs`

- Both `.filter(|unit| unit.kind == "user")` sites (the units layer pool near line 292 and the second near line 1061) become `.filter(|unit| crate::store::in_answer_pool(&unit.kind))`.
- In the `unit_winner` branch (near line 642) the `Answer` gets `source_kind: top.kind.clone()` and the reason is `units layer answer (user's own words), confidence capped medium` when `top.kind == "user"` and `units layer answer (agent capture), confidence capped medium` when `top.kind == "capture"`. `UnitHit` already carries `kind`.
- Nothing else changes; `kind_rank`, `dedupe_user_units`, `is_boilerplate_unit` apply to capture units as they do to user units.

Test: `ask_answers_from_a_capture_unit_with_capture_provenance`: a store with one capture-kind unit (build it with `crate::ingest::capture_unit`) answers a query from its distinctive words with `layer == "unit"`, `source_kind == "capture"`, the agent-capture reason, verdict `answered`, confidence `medium`. Keep every existing test unchanged.

### 4.3 `plugins/qol-memory/src/app/warm.rs`

The `.filter(|unit| unit.kind == "user")` near line 163 becomes `.filter(|unit| crate::store::in_answer_pool(&unit.kind))`. The warm layer must serve capture units pushed by `push_units` after a daemon capture; verify by reading `push_units` and adjust only if it filters by kind itself.

### 4.4 `plugins/qol-memory/src/continue_recall/mod.rs`

- `is_candidate`: the kind gate accepts `user`, `compaction` and `capture`.
- The slot loop: a `capture` unit takes the user slot (`index 0`, `CAP_USER`); `compaction` stays in slot 1.
- The printed block keeps its existing shape; the kind column shows `capture` for these units.

Test: `continue_injects_capture_units_like_user_units` in the existing test style (a capture unit newer than the marker with text longer than `MIN_TEXT` is picked; a compaction unit still takes the second slot).

### 4.5 `docs/research/qol-memory/ask.mjs`

Mirror 4.2 so the JS reference stays the parity oracle: the `userUnits` filter near line 113 becomes `u.kind === "user" || u.kind === "capture"`; in the unit-winner branch near line 318 `source_kind: unitTop.kind` and the reason picks the agent-capture wording when `unitTop.kind === "capture"`. No other JS file changes (`snapshot.mjs`, `notes.mjs`, `replay.mjs` keep their own filters; captures are settled facts and are not distillation input).

## 5. Gate and acceptance (architect)

1. `cargo fmt --all --check`; `cargo clippy -p qol-memory --all-targets -- -D warnings` on host, `x86_64-apple-darwin`, `x86_64-pc-windows-msvc`; `cargo test -p qol-memory`; `env -u QOL_TRAY_HTTP_TOKEN cargo run -q -p qol -- check`.
2. Parity: `node docs/research/qol-memory/parity.mjs --bin target/debug/qol-memory --store <copy>` reports 73/73.
3. CLI: on a store copy, `qol-memory capture --text '<fact>' --cwd /x` prints `appended: 1`, again prints `appended: 0`, `qol-memory ask '<distinctive words>' --store <copy>` answers from the unit with `source_kind capture`; `qol-memory continue --cwd /x --session s --store <copy>` lists the capture unit.
4. Live: recompile-self, `tools/list` shows the three tools, `tools/call qol-memory__capture` with a real fact answers appended 1 then 0, `tools/call qol-memory__ask` recalls it.
5. Squash to main as one commit with explicit paths; no push.

## 6. Out of scope

- Wiring the tray MCP endpoint into Claude Code, pi and Codex configs (Phase 4, qol-skills bridge). Until then agents reach `capture` only through the CLI.
- Changing `qol_mcp::input_schema` optionality.
- Distilling capture units into notes.
