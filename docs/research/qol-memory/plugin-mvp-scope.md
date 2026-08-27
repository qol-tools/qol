# qol-memory plugin MVP - build spec v1

Status: accepted 2026-08-27 (user). Engine = Rust port of the read path. Daemon deferred to stage 2.

## What the MVP is

A monorepo plugin `plugins/qol-memory` (id `qol-memory`) whose binary `qol-memory` answers
`qol-memory ask "<query>"` from the existing store with output that is semantically identical to
`node docs/research/qol-memory/ask.mjs "<query>"`. The Node scripts stay the write-path tooling
(snapshot, notes, decisions, ingest, live capture) and the eval harness. The plugin owns the read
path, the store location contract, `status`, and `doctor`.

Acceptance gate (architect runs it, lanes never do):

1. `cargo fmt --check`, `cargo clippy -p qol-memory --all-targets -- -D warnings`, `cargo test -p qol-memory`, `cargo run -q -p qol -- check`.
2. `node docs/research/qol-memory/parity.mjs` passes: every question in `eval/questions.json`,
   `eval/heldout.json`, and `eval/skills-questions.json`, brief and full mode, both cache directions
   (JS-written cache read by Rust, Rust-written cache read by JS), deep-equal after parsing.
3. Frozen eval numbers unchanged: `node eval/eval.mjs` units bm25 hit@1 8/30, hit@5 11/30, mrr 0.340.
4. `qol-memory doctor --json` valid on this host; `qol-memory help`, `help ask`, `ask help` work.

## Verified facts the port relies on

- The store is `qol_config::data_subdir("plugins/qol-memory")` = `$XDG_DATA_HOME/qol-tray/plugins/qol-memory`,
  overridable by `QOL_MEMORY_STORE`, then by `--store PATH` (highest). Same resolver as `lib/store-path.js`.
- Store files read by `ask`: `units.jsonl` (+ optional `units.seal.json` / `units.seal.gz`), `notes/<run>/notes.jsonl`
  (newest run by name), `skills/index.json`, `idx-<layer>.json` + `.meta` caches. Written by `ask`: the caches
  and `retrievals.jsonl`. `manifest.json` is written by ask.mjs only; the Rust binary never touches it.
- `qol-headless` gives `Command::run_plain_text` and `Command::run_json` on the same command, a global `--json`
  flag (before or after the command path), `help` routing, and `DoctorCheck`/`DoctorCheckResult` (read
  `libs/qol-headless/src/doctor/` for the exact API and `DoctorStatus` variants). `CommandContext::args()` carries
  the tokens after the command path; confirm in `libs/qol-headless/src/lib.rs` before parsing.
- Deps already in `Cargo.lock`: `sha1 0.10`, `flate2 1`, `regex 1`, `serde`, `serde_json` (workspace). Nothing new
  is compiled into the workspace.
- ask.mjs line 153 calls `readFile` which is not imported; the skills layer therefore always reports
  `not-indexed`. That is a bug (silent degradation). Lane E fixes it to `readFileSync`; the Rust port implements the
  fixed semantics.
- ask.mjs crashes (line 261, `noteResolved.key` on null) when the store has no notes run. The port must not crash:
  an empty notes layer means no note candidate, verdict falls through to the units/no-memory logic.
- JS string semantics that affect scores: `text.length`, `slice`, and `indexOf` are UTF-16 code units. Doc length in
  BM25 is the UTF-16 length. The port must use UTF-16 lengths and indices where the JS does.
- `Number(x.toFixed(2))` rounds half up on the exact decimal expansion of the double (0.625 -> 0.63, 0.125 -> 0.13).
  Rust `format!("{:.2}")` rounds half to even (0.625 -> 0.62). The port ships `to_fixed2` with JS semantics.
- Tokenization regex is `/[\p{L}\p{N}]+/gu` on the lowercased text. Rust `char::is_alphanumeric` is a superset
  (Other_Alphabetic marks). Use the `regex` crate with `[\p{L}\p{N}]+` for exact parity. JS `\d` is ASCII only:
  use `[0-9]` in ported regexes, never `\d`.
- Query strings are re-tokenized inside `bm25Ranks`. ask.mjs joins already-normalized tokens with spaces and
  passes the string in, so query tokens are normalized twice (`houses` -> `hous` -> `hou`). The port must do the
  same: `bm25_ranks` takes a `&str` and tokenizes it; callers pass the joined string exactly as ask.mjs builds it.

## Crate layout (qol-arch-code)

```text
plugins/qol-memory/
  plugin.toml  Cargo.toml  build.rs  README.md  LICENSE  .gitignore
  assets/concept-aliases.json      moved from docs/research/qol-memory/ (Lane E)
  assets/skills-glossary.json      moved from docs/research/qol-memory/ (Lane E)
  src/main.rs                      template shape: cli::exit_code(args) + manifest test
  src/cli.rs                       HeadlessApp wiring only (ask, status, doctor checks, help text)
  src/platform/{mod,linux,macos,windows,fallback,support}.rs   copied from the template unchanged
  src/text/mod.rs                  tokens, normalize, utf16 helpers, to_fixed2, iso time
  src/store/mod.rs                 Store (root resolution), Unit, Note, layers, dedupe, boilerplate
  src/store/seal.rs                seal marker + gunzip prefix (lib/seal.js read side)
  src/retrieval/mod.rs             Index, build_index, bm25_ranks, snippet
  src/retrieval/cache.rs           persisted index cache (lib/indexcache.js), same on-disk schema
  src/aliases/mod.rs               concept aliases (lib/concept-aliases.js) + embedded asset
  src/skills/mod.rs                skills pool read side (lib/skills-pool.js minus walkSkills)
  src/retrieval_log/mod.rs         retrievals.jsonl append/rotate/candidates (lib/retrieval-log.js)
  src/ask/mod.rs                   AskRequest, Gates, AskOutput and the verdict engine (ask.mjs 104-380)
  src/doctor/mod.rs                plugin doctor checks
```

Dependency direction: `cli -> ask -> {store, retrieval, aliases, skills, retrieval_log} -> text`. `doctor -> store, aliases, skills, retrieval::cache`. Nothing depends on `cli`.

### plugin.toml

```toml
[plugin]
id = "qol-memory"
uid = "a2d9cb99-1d62-4f73-ab8e-51de1fa7ac63"
name = "QoL Memory"
description = "Long-context memory: retrieve settled facts from your agent session history"
version = "0.1.0"
author = "KMRH47"
platforms = ["linux", "macos"]

[runtime]
command = "qol-memory"

[action.status]
label = "Status"
args = ["status"]

[capabilities]
doctor = true

[menu]
label = "QoL Memory"
items = []

[[dependencies.binaries]]
name = "qol-memory"
repo = "qol-tools/qol-memory"
pattern = "qol-memory-{os}-{arch}"
```

No `qol-config.toml`, no settings action, no daemon, no gpui in the MVP.

### Cargo.toml

```toml
[package]
name = "qol-memory"
version = "0.1.0"
edition = "2021"

[dependencies]
anyhow = "1"
flate2 = "1"
regex = "1"
sha1 = "0.10"
sha2.workspace = true
serde.workspace = true
serde_json.workspace = true
qol-config.workspace = true
qol-conventions.workspace = true
qol-fs.workspace = true
qol-headless.workspace = true

[build-dependencies]
qol-conventions.workspace = true

[dev-dependencies]
qol-plugin-api.workspace = true
```

## CLI contract

```text
qol-memory ask "<query>" [--k N] [--exclude-session ID] [--brief] [--log-source S] [--log-cwd PATH] [--log-fact FACT] [--no-log] [--store PATH]
qol-memory status [--store PATH]
qol-memory doctor
qol-memory help | help <cmd> | <cmd> help
```

- `--json` is the global qol-headless flag; `ask` and `status` register a `run_result` (plain text, usage errors
  exit 64) and a `run_json` handler. `--json ask ...` prints exactly the `AskOutput` object (compact, as
  qol-headless prints JSON) and nothing else on stdout.
- Round 1 accepted deviations: `src/lib.rs` exists (module tree) and `main.rs` calls `qol_memory::cli::exit_code`;
  `run_and_log` measures `latency_ms` from its own entry; `SkillHit.truncated/hash_match` and `NoteOut.text` are
  `Option` so brief mode can omit them.
- Argument parsing for `ask`: walk `ctx.args()`; value flags (`--k`, `--exclude-session`, `--log-source`,
  `--log-cwd`, `--log-fact`, `--store`) consume the next token; `--brief`, `--no-log` are booleans; the first
  remaining token is the query; a second positional or an unknown `--flag` is a usage error (exit 64) with the usage
  line. Missing query is a usage error. Defaults: k=5, log-source `ask-cli`.
- Plain-text `ask` output: line 1 `verdict: <verdict> (<confidence>)`, line 2 `reason: <reason>`, then when
  answered `answer [<layer>/<cls or ->]: <text>`, then `recalled:` followed by one line per recalled note
  `  <key>  <cls>  <score>`; when `skills.hits` is non-empty, `skills:` with `  <id>  <score>  <section>`.
- `status` JSON: `{ store, exists, units_file: {present, bytes, sealed}, notes_run, index_caches: {pool, user, notes}
  each "fresh"|"stale"|"missing", skills: {present, root, head, dirty, walked_at}, retrievals: {bytes, last_ts},
  candidates_pending }`. "fresh" means the `.meta` prefix proof `fp` matches the current units file (see cache
  section); for `notes` it means the fingerprint file exists (no source proof). Plain-text is one `key: value` per line.
- `default_command(["status"])`.
- The binary never writes `manifest.json`.

## Module contracts

All signatures are the interface between lanes; keep them exactly so parallel work composes. Bodies may add
private helpers. Every module gets unit tests for the JS-parity corners named below (tests live in the module).

### src/text/mod.rs

```rust
pub fn tokens(text: &str) -> Vec<String>;
pub fn normalize(token: &str) -> String;
pub fn utf16_len(text: &str) -> usize;
pub fn utf16_slice(text: &str, start: usize, end: usize) -> String;
pub fn utf16_index_of(haystack: &str, needle: &str) -> Option<usize>;
pub fn to_fixed2(value: f64) -> f64;
pub fn parse_iso_millis(ts: Option<&str>) -> i64;
pub fn now_iso() -> String;
pub fn collapse_ws_lower(text: &str) -> String;
pub fn run_dir_millis(run: &str) -> i64;
```

- `tokens`: lowercase (`str::to_lowercase`), match `[\p{L}\p{N}]+` (regex crate, compiled once in a
  `std::sync::OnceLock`), keep matches whose char count > 1, map `normalize`. Order preserved, duplicates kept.
- `normalize`: exact port of `lib/retrieval.js` lines 7-19. Lengths are char counts (all branches only fire on
  ASCII suffixes, so UTF-16 and char counts agree for the suffix checks; use `chars().count()`).
- `utf16_len`: `text.encode_utf16().count()`. `utf16_slice(text, start, end)`: JS `text.slice(start, end)` with
  clamping, decoded with `String::from_utf16_lossy`. `utf16_index_of`: JS `indexOf` result in UTF-16 units.
- `to_fixed2`: JS `Number(x.toFixed(2))`. Decompose the f64 into sign, mantissa `m`, exponent `e` (`x = m * 2^e`),
  compute `round_half_up(|x| * 100)` exactly with u128 integer arithmetic (when `-e > 100` the value is below
  0.005 and the result is 0), reapply the sign, divide by 100.0. NaN and infinities pass through unchanged. Tests:
  0.625 -> 0.63, 0.125 -> 0.13, 1.005 -> 1.0, 8.345 -> 8.35 (verify each expected value with `node -e` before
  writing the test), 2.5 -> 2.5, 0.0 -> 0.0.
- `parse_iso_millis`: JS `new Date(ts || 0).getTime()`. Parse `YYYY-MM-DDTHH:MM:SS(.fff)?Z` (also accept `+00:00`
  as Z); None, empty, or unparsable -> 0. Days-from-civil arithmetic, no chrono.
- `now_iso`: `2026-08-27T08:39:05.554Z` shape (millisecond precision, UTC) from `SystemTime`.
- `collapse_ws_lower`: lowercase, runs of whitespace (`char::is_whitespace` or `\u{FEFF}`) -> single space, trim.
- `run_dir_millis`: ask.mjs `runTime`: `2026-08-10T21-38-02-273Z` -> `2026-08-10T21:38:02.273Z` -> millis.

### src/store/mod.rs and src/store/seal.rs

```rust
pub struct Store { root: PathBuf }
impl Store {
    pub fn resolve(explicit: Option<&Path>) -> anyhow::Result<Store>;
    pub fn root(&self) -> &Path;
    pub fn units_path(&self) -> PathBuf;
    pub fn snapshot_root(&self) -> PathBuf;
    pub fn notes_root(&self) -> PathBuf;
    pub fn skills_index_path(&self) -> PathBuf;
    pub fn retrievals_path(&self) -> PathBuf;
    pub fn candidates_path(&self) -> PathBuf;
    pub fn read_units(&self) -> anyhow::Result<UnitsLayer>;
    pub fn read_notes(&self) -> anyhow::Result<NotesLayer>;
}
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Unit { pub key: String, #[serde(default)] pub source: Option<String>, #[serde(default)] pub file: Option<String>,
    #[serde(default)] pub session: Option<String>, #[serde(default)] pub cwd: Option<String>, #[serde(default)] pub kind: String,
    #[serde(default)] pub ts: Option<String>, #[serde(default)] pub text: String }
pub struct UnitsLayer { pub run: String, pub path: PathBuf, pub items: Vec<Unit> }
#[derive(Clone, Debug, serde::Deserialize)]
pub struct Note { pub key: String, #[serde(default)] pub cls: String, #[serde(default)] pub text: String,
    #[serde(default)] pub source_key: Option<String>, #[serde(default)] pub source_ts: Option<String>,
    #[serde(default)] pub source_kind: Option<String> }
pub struct NotesLayer { pub run: Option<String>, pub items: Vec<Note> }
pub const BOILERPLATE_MARKERS: [&str; 4];
pub fn dedupe_user_units(units: &[Unit]) -> Vec<Unit>;
pub fn is_boilerplate_unit(unit: &Unit) -> bool;
pub fn parse_units_text<T: serde::de::DeserializeOwned>(text: &str) -> Vec<T>;

pub mod seal {
    pub const SEAL_SCHEMA: &str = "qol-memory-seal-v1";
    pub fn try_sealed_text(root: &Path, raw: &[u8]) -> Option<String>;
}
```

- `resolve`: explicit > `QOL_MEMORY_STORE` (non-empty) > `qol_config::data_subdir("plugins/qol-memory")`; error
  when none resolves. Never creates the directory.
- `read_units`: if `units.jsonl` exists, read bytes, `try_sealed_text` else lossy utf8, `run = "live"`,
  `path = units_path`. Otherwise newest `snapshot/<run>/snapshot.jsonl` where run dirs match `^[0-9]{4}-[0-9]{2}-[0-9]{2}T`
  (sorted by name, last); error `no runs under <snapshot_root>` when none.
- `parse_units_text`: split on `\n`, skip empty lines, skip lines that fail to parse (ask.mjs `parseUnitsText`).
- `read_notes`: newest run dir under `notes/` by the same name rule; `run: None, items: []` when none; every
  non-empty line must parse (an unparsable note line is an error, like JS `JSON.parse` throwing).
- `dedupe_user_units`: input is already filtered to `kind == "user"` by the caller. Stable sort by
  `parse_iso_millis(ts)` ascending, then keep the first unit per `collapse_ws_lower(text)`.
- `is_boilerplate_unit`: any marker is a substring of `text`. Markers from ask.mjs lines 39-44.
- `try_sealed_text`: exact port of `lib/seal.js` `trySealedText`: marker must parse with `schema == SEAL_SCHEMA`,
  `prefix_len` integer in `0..=raw.len()`, `blob_len` equals the blob file size, gunzip (flate2 `GzDecoder`) length
  must equal `prefix_len`; result is `prefix ++ raw[prefix_len..]` as lossy utf8. Any failure -> None.

### src/retrieval/mod.rs and src/retrieval/cache.rs

```rust
pub struct DocRef<'a> { pub key: &'a str, pub text: &'a str }
pub struct IndexDoc { pub key: String, pub tf: HashMap<String, u32>, pub len: usize }
pub struct Index { pub docs: Vec<IndexDoc>, pub idf: HashMap<String, f64>, pub df: HashMap<String, u32>, pub n: usize, pub avgdl: f64, pub total_length: usize }
pub struct Ranked { pub key: String, pub score: f64 }
pub fn build_index(items: &[DocRef<'_>]) -> Index;
pub fn bm25_ranks(query: &str, idx: &Index, k: usize) -> Vec<Ranked>;
pub fn snippet(text: &str, match_words: &[String], window: usize) -> String;

pub mod cache {
    pub fn persisted_index_path(root: &Path, layer: &str) -> PathBuf;
    pub fn layer_fingerprint(items: &[DocRef<'_>]) -> String;
    pub fn build_or_load(root: &Path, layer: &str, items: &[DocRef<'_>], source_path: Option<&Path>) -> Index;
    pub fn cache_state(root: &Path, layer: &str, items: &[DocRef<'_>], source_path: Option<&Path>) -> CacheState;
    pub enum CacheState { Fresh, Stale, Missing }
}
```

- `build_index`: `len = utf16_len(text)`; tf counts over `tokens(text)`; df counts docs per term; `n`, `avgdl =
  total_length / max(1, n)` (f64), `idf[t] = ln(1 + (n - df + 0.5) / (df + 0.5))`.
- `bm25_ranks`: `qt = tokens(query)`; empty -> empty. For each doc in `docs` order, `s = sum over qt in order` of
  `(idf * f * 1.2) / (f + 1.2 * (1 - 0.75 + 0.75 * (len / avgdl)))` for terms with `f > 0`, idf default 0. Every doc is
  scored (including 0). Sort by score descending, ties by key ascending (byte order). `k == 0` returns all, else the
  first k. Use `sort_by` with `partial_cmp` (scores are finite).
- `snippet`: port of `lib/retrieval.js` 56-71 in UTF-16 units: lower = `text.to_lowercase()`; idx = min over match
  words of `utf16_index_of(lower, w)` that are found; not found -> `utf16_slice(text, 0, window)`; else `start = max(0,
  idx - window/3)` (integer division), `s = utf16_slice(text, start, start + window)` with whitespace runs collapsed
  to one space and trimmed, prefix `...` when start > 0, suffix `...` when `start + window < utf16_len(text)`.
  Default window 240 is the caller's business.
- `cache`: same on-disk schema as `lib/indexcache.js` so both implementations share the files:
  `idx-<layer>.json` = `{ "N", "avgdl", "totalLength", "terms": [..], "idfArr": [..], "dfArr": [..], "rows": [{"k", "L", "tf": [id, f, id, f, ...]}] }`,
  `idx-<layer>.json.meta` = `{ "fp", "size", "count", "firstKey", "lastKey", "fingerprint" }` when a source path is
  given, else `{ "fingerprint" }`. Term id order is free (the file is self-consistent); everything else must match.
  - `layer_fingerprint`: sha1 over the concatenation, for each item, of `key` then the decimal `utf16_len(text)`
    (0 for empty), then the decimal item count; hex, first 16 chars. `prefix proof`: sha1 over
    `"{size}:{count}:{firstKey}:{lastKey}"` where size is the source file byte size and first/last keys are the first
    and last item keys (empty strings when no items); `fp` = first 16 hex chars. Both must match the JS byte for byte
    (the parity harness compares `.meta` files).
  - `build_or_load` decision order is the exact port of `buildOrLoad` (lines 160-186): fp match -> load; canMerge
    (`proof.size > meta.size && items.len() >= meta.count && (meta.count == 0 || (items[meta.count-1].key ==
    meta.lastKey && items[0].key == meta.firstKey))`) -> load cached, then when `items.len() == meta.count` and the
    fingerprint matches, rewrite meta with the new fp and return cached, else `merge_tail` (append the tail docs,
    update df, recompute every idf with the merged N, recompute avgdl and total) then `save_index`; then fingerprint
    match -> load; any error in that block -> fall through to `build_index` + `save_index`. Loading a cache yields
    docs with `tf` and `len` only (keys come from rows).
  - `save_index` writes the JSON, writes the meta, then prunes `idx-pool-x-*.json` caches beyond the 5 newest by
    mtime (unlink both the json and its `.meta`, ignoring errors). Write with `qol_fs::atomic_write` when its
    signature fits, else tmp + rename.
  - `cache_state`: Fresh when a meta exists and (proof fp matches, or no source path and the fingerprint matches),
    Stale when a meta exists but neither matches, Missing otherwise. Used by `status` and `doctor`.

### src/aliases/mod.rs

```rust
pub const ALIAS_CAP: usize = 4;
pub const CONCEPT_ALIASES_JSON: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/concept-aliases.json"));
#[derive(Default)] pub struct AliasMap { map: HashMap<String, Vec<String>> }
impl AliasMap { pub fn get(&self, term: &str) -> Option<&[String]>; pub fn len(&self) -> usize; pub fn is_empty(&self) -> bool; }
pub fn embedded() -> AliasMap;
pub fn load(json: &str) -> anyhow::Result<AliasMap>;
pub fn validate(json: &str) -> Vec<String>;
pub fn expand_tokens(list: &[String], map: &AliasMap) -> Vec<String>;
pub fn expand_tokens_keep(list: &[String], map: &AliasMap) -> Vec<String>;
```

- `load`: `schema` must be 1; for each `term -> [expansions]`, flatten `tokens(expansion)` in order, stop at
  `ALIAS_CAP` tokens total per term. `embedded()`: empty map when `QOL_MEMORY_ALIASES_DISABLE == "1"`, else `load`
  of the embedded asset; a load failure prints one line to stderr (`concept-aliases: load failed: <err>; using empty
  alias map`) and returns empty.
- `validate`: port of `lib/concept-aliases.js` 53-78 (term regex `^[a-z0-9]{2,}$`, messages verbatim).
- `expand_tokens`: replace an aliased token by its expansions; `expand_tokens_keep`: keep the token and append its
  expansions.

### src/skills/mod.rs

```rust
#[derive(serde::Deserialize)] pub struct SkillsIndex { pub schema: u32, #[serde(default)] pub walked_at: Option<f64>, #[serde(default)] pub root: Option<String>,
    #[serde(default)] pub repo: Option<Repo>, #[serde(default)] pub skills: Vec<SkillMeta> }
#[derive(serde::Deserialize)] pub struct Repo { #[serde(default)] pub name: Option<String>, #[serde(default)] pub head: Option<String>, #[serde(default)] pub dirty: Option<bool> }
#[derive(serde::Deserialize)] pub struct SkillMeta { pub id: String, #[serde(default)] pub name: String, #[serde(default)] pub description: String, #[serde(default)] pub title: String,
    pub rel: String, #[serde(default)] pub hash: String, #[serde(default)] pub bytes: u64, #[serde(default)] pub sections: Vec<Section>, #[serde(default)] pub aliases: Vec<String> }
#[derive(serde::Deserialize)] pub struct Section { pub h: String, #[serde(default)] pub lead: String }
pub enum Freshness { NotIndexed, Unavailable, Stale, Fresh }
impl Freshness { pub fn as_str(&self) -> &'static str; }
pub fn load_index(path: &Path) -> Option<SkillsIndex>;
pub fn pool_tokens(text: &str) -> Vec<String>;
pub fn build_meta_doc(skill: &SkillMeta) -> String;
pub fn probe_fresh(index: &SkillsIndex, root: &Path) -> Freshness;
pub struct SplitSection { pub h: String, pub text: String }
pub fn split_sections(raw: &str) -> Vec<SplitSection>;
pub struct BestSection { pub h: String, pub text: String, pub score: f64 }
pub fn best_section(skill: &SkillMeta, root: &Path, qtokens: &[String], idf: &HashMap<String, f64>, cap: usize) -> Option<BestSection>;
pub enum Served { Ok { content: String, section: String, truncated: bool, hash_match: bool, live_hash: String }, Failed { reason: String } }
pub fn serve_section(skill: &SkillMeta, root: &Path, header_hint: Option<&str>, cap: usize) -> Served;
```

Exact ports of `lib/skills-pool.js` (`STEM_MAP`, `poolTokens`, `buildMetaDoc`, `probeFresh` with `mtime > walked_at`
in millis, `splitSections`, `bestSection` with `idf.get(t).filter(|w| *w != 0.0).unwrap_or(1.0)` weights and the
`si == 0` intro penalty 0.5, `serveSection` with `norm = lowercase, strip backticks, trim`, fallback target = first
section whose text length is in `24..=400` UTF-16 units, live hash = sha256 hex first 16 via the `sha2` workspace
crate (it must match `hash` written by skills.mjs). `content = text.trim()[..cap]` in UTF-16 units,
`truncated = trimmed len > cap`.
`load_index`: None when missing or unparsable.

### src/retrieval_log/mod.rs

```rust
pub const RETRIEVAL_SCHEMA: &str = "qol-memory-retrieval-v1";
pub const CANDIDATES_SCHEMA: &str = "qol-memory-candidates-v1";
pub const RETRIEVAL_LOG_CAP: u64 = 10 * 1024 * 1024;
pub const RETRIEVAL_LOG_TAIL: u64 = 1024 * 1024;
pub fn normalize_query(s: &str) -> String;
pub fn candidate_key(norm_query: &str) -> String;
pub fn rotate_if_needed(path: &Path, cap: u64, tail: u64);
pub fn correctness_of(verdict: &str, answer_text: Option<&str>, fact: Option<&str>, source: &str) -> Option<String>;
#[derive(serde::Serialize)] pub struct RetrievalEvent { pub ts: String, pub source: String, pub session: Option<String>, pub cwd: Option<String>, pub query: String,
    pub verdict: String, pub confidence: String, pub correctness: Option<String>, pub latency_ms: u64, pub k: usize, pub exclusion: Exclusion,
    pub gates: serde_json::Value, pub signals: serde_json::Value, pub answer_key: Option<String>, pub recalled_keys: Vec<String>, pub counts: serde_json::Value }
#[derive(serde::Serialize)] pub struct Exclusion { pub exclude_session: bool, pub non_default_gates: bool }
pub fn append(root: &Path, event: &RetrievalEvent);
pub fn count_pending_candidates(root: &Path) -> usize;
pub fn last_event_ts(root: &Path) -> Option<String>;
```

- `normalize_query`: lowercase, every char not in `[a-z0-9 ]` -> space, collapse spaces, trim. `candidate_key`:
  sha256 hex first 16 (needs `sha2`). `rotate_if_needed`: when the file is larger than cap keep the tail from the
  first line boundary inside the last `tail` bytes (port of lines 34-47).
- `correctness_of`: port of `correctnessOf` (fact None -> None; eval + `trap:` prefix -> `trapped` when answered else
  `untrapped`; not answered -> `unanswered`; normalized fact empty -> `correct`; normalized answer contains
  normalized fact -> `correct` else `wrong`).
- `append`: no-op when `QOL_MEMORY_RETRIEVAL_LOG_DISABLE == "1"`; create the root dir, rotate, append one JSON line.
  A failure writes one line to stderr and returns (never fails the query).
- `count_pending_candidates`: lines of `candidates.jsonl` parsing as objects with `status == "candidate"`.
- `last_event_ts`: `ts` of the last parsable line of `retrievals.jsonl` (used by `status`).

### src/ask/mod.rs

```rust
pub struct AskRequest { pub query: String, pub k: usize, pub brief: bool, pub exclude_session: Option<String> }
pub struct LogOptions { pub source: String, pub cwd: Option<String>, pub fact: Option<String>, pub no_log: bool }
#[derive(Clone, Copy, serde::Serialize)] pub struct Gates { #[serde(rename = "NO_MEMORY_COV")] pub no_memory_cov: f64, #[serde(rename = "FLOOR")] pub floor: f64,
    #[serde(rename = "NOTE_COV")] pub note_cov: f64, #[serde(rename = "NOTE_SCORE")] pub note_score: f64, #[serde(rename = "UNIT_COV")] pub unit_cov: f64,
    #[serde(rename = "UNIT_SCORE")] pub unit_score: f64, #[serde(rename = "UNIT_MARGIN")] pub unit_margin: f64, #[serde(rename = "HIGH_MARGIN")] pub high_margin: f64 }
impl Gates { pub const DEFAULTS: Gates; pub fn from_env() -> Gates; pub fn is_default(&self) -> bool; }
pub fn run(store: &Store, aliases: &AliasMap, req: &AskRequest) -> anyhow::Result<AskOutput>;
pub fn run_and_log(store: &Store, aliases: &AliasMap, req: &AskRequest, log: &LogOptions) -> anyhow::Result<AskOutput>;
pub fn render_text(out: &AskOutput) -> String;
```

`AskOutput` and its children are `serde::Serialize` structs whose field order and null/absent behaviour reproduce
ask.mjs lines 344-380 exactly:

```text
AskOutput { query, verdict, confidence, reason, gates: Gates, non_default_gates: bool, answer: Option<Answer> (null when none),
            recalled: Vec<Recalled>, related: Vec<Related>, signals: Signals, counts: Counts, skills: SkillsOut,
            units: Option<Vec<UnitOut>> (absent in brief: skip_serializing_if none), notes: Vec<NoteOut> }
Answer { text, layer ("note"|"unit"), key, cls: Option<String> (null for unit), source_kind, source_ts: Option<String>,
         session: Option<String> (absent for note answers: skip_serializing_if none),
         score: f64, margin: Option<f64> (null for unit),
         superseded: Option<Option<Vec<Superseded>>> (absent for unit answers; null for a note answer without superseded; skip_serializing_if outer none) }
Superseded { text, source_ts: Option<String> }
Recalled { key, cls, score, source_kind: Option<String> (skip if none), source_ts: Option<String> (skip if none) }
Related { text, cls, source_ts: Option<String> }
Signals { top_note_score: Option<f64>, top_unit_score: Option<f64>, unit_margin: Option<f64>, note_token_coverage: f64, unit_token_coverage: f64,
          max_token_coverage: f64, notes_run_ts: Option<String>, snapshot_run_ts: String, live_units: bool, stale_layer: bool, recency_resolved: bool }
Counts { units: usize, notes: usize }
SkillsOut { status: String, root: Option<String>, head: Option<String>, dirty: Option<bool>, hits: Vec<SkillHit> }
SkillHit { id, name (skip in brief), score, section: Option<String>, content: Option<String> (skip in brief), truncated: bool (skip in brief),
           hash_match: bool (skip in brief), status, head: Option<String> (skip in brief), dirty: Option<bool> (skip in brief) }
UnitOut { key, score, kind, text, source: Option (skip if none), session: Option (skip if none), cwd: Option (skip if none), ts: Option (skip if none), snippet }
NoteOut brief: { key, cls, score, text (only when verdict == "answered", skip otherwise) }
NoteOut full:  { key, cls, text, source_key: Option (skip if none), source_ts: Option (skip if none), source_kind: Option (skip if none), score }
```

Every `score`, `margin`, coverage, and signal number goes through `to_fixed2` exactly where ask.mjs applies
`toFixed(2)`; raw f64 elsewhere (gates). `unit_margin` when the top unit has no runner-up is `Infinity` in JS and
`Number((Infinity).toFixed(2))` is `Infinity`, which `JSON.stringify` renders as `null`: serialize non-finite f64 as
null (serde_json does this for f64 by default; keep it). `margin` uses `min(margin, 99)` before rounding.

`run` is the port of ask.mjs lines 104-380 in this order:

1. `units = store.read_units()`; `user_units = dedupe_user_units(units filtered kind == "user")`; `notes = store.read_notes()`.
2. `qtokens0 = tokens(query)` minus `STOPWORDS` (ask.mjs line 35, verbatim set); `qtokens = expand_tokens(qtokens0)`.
3. `answer_pool = user_units` minus boilerplate minus `session == exclude_session`; `answer_idx =
   cache::build_or_load(root, layer, answer_pool, Some(units.path))` with layer `pool` or `pool-x-<first 8 chars of the session id>`.
   `units_query = expand_tokens_keep(tokens(query)).join(" ")`; `answer_ranked = bm25_ranks(units_query, answer_idx, k)`
   joined back to the pool units by key.
4. `all_idx = build_or_load(root, "user", user_units, Some(units.path))`; `ranked_all = bm25_ranks(units_query, all_idx, k)`;
   `top_units = ranked_all + snippet(text, qtokens, 240)`.
5. `notes_idx = build_or_load(root, "notes", notes, None)` when notes is non-empty; `top_notes =
   bm25_ranks(expand_tokens(qtokens0).join(" "), notes_idx, 5)` joined to notes by key.
6. Skills: `load_index(store.skills_index_path())`; `skills_root = index.root or QOL_MEMORY_SKILLS_ROOT env or None`;
   `freshness = probe_fresh` when an index exists else NotIndexed; when the index has skills: `meta_docs = (id,
   build_meta_doc)`, `skills_idx = build_index`, `qt = pool_tokens(query)` minus STOPWORDS, `ranked = bm25_ranks(query, skills_idx, 5)`
   (raw query), dedupe by id, for each: `best = best_section(s, root, qt, skills_idx.idf, 2048)`, `served =
   serve_section(s, root, best.h or None, 2048)`, hit fields per ask.mjs 173-185 (`section = served.section when ok
   else best.h`, `status = "served" when ok else reason`). Skills `status` out = `served` when Fresh, else the
   freshness string, `not-indexed` without an index. When `skills_root` is None every `best_section`/`serve_section`
   is skipped and hits stay empty.
7. Verdict engine: port lines 189-332 verbatim, with these named helpers: `distinct_score(qt, text) -> (matched,
   total)` (substring test on the lowercased text, duplicates counted), `phrased_coverage`, `weighted_note_cov`,
   `family_key(note)` (head = text up to the first `" | "`; `[0-9]+` -> `#`; lowercase; strip a trailing
   ` in the corpus`; strip a trailing `\(.*\)$` (regex crate, dot excludes newline); trim; `cls + ":" + first 60
   UTF-16 units`), `RECENCY_CLS`, `STALE_CLS`, `CURATED_KINDS`, `KIND_RANK` (unknown kind -> 0), gate values from
   `Gates::from_env()` (env `MEM_NO_COV`, `MEM_FLOOR`, `MEM_NOTE_COV`, `MEM_NOTE_SCORE`, `MEM_UNIT_COV`,
   `MEM_UNIT_SCORE`, `MEM_UNIT_MARGIN`, `MEM_HIGH_MARGIN`; unparsable env -> the default). Empty notes: `note_top`,
   `note_resolved` are None, `note_decisive = true`, `note_cov = note_cov_r = 0`, `has_recency_answer = false`,
   `note_winner = false`; the rest of the logic is unchanged. Exact float equality where the JS uses `===`.
8. `live_units = units.run == "live"`; `stale_layer = !live && notes.run.is_some() && run_dir_millis(notes.run) < run_dir_millis(units.run)`.
9. `run_and_log` measures latency from process start (pass `Instant` from `cli`), builds `RetrievalEvent` with
   `gates`/`signals`/`counts` serialized from the output, `session = exclude_session`, `answer_key`, `recalled_keys`,
   and calls `retrieval_log::append` unless `no_log`.

### src/doctor/mod.rs

```rust
pub fn checks() -> Vec<qol_headless::DoctorCheck>;
```

Checks (ids, in order): `platform_supported` (template), `store_dir` (resolvable and exists: ok; resolvable but
missing: warn "no memory yet", fix "run a session with live capture or ingest a snapshot"; unresolvable: fail),
`units_layer` (units.jsonl or a snapshot run: ok with the unit count from the file line count; none: warn),
`notes_layer` (a notes run: ok naming the run; none: warn), `index_cache` (`cache_state` for `user` against the live
units: Fresh ok, Stale warn "rebuilt on next ask", Missing warn), `skills_index` (Fresh ok; Stale warn, fix
`node docs/research/qol-memory/skills.mjs`; Unavailable warn; missing warn), `retrieval_log` (size under cap ok,
else warn), `aliases_valid` (`aliases::validate(CONCEPT_ALIASES_JSON)` empty: ok; else fail listing the errors).
Use `DoctorCheckResult::ok/warn/fail` as the qol-headless API names them; read `libs/qol-headless/src/doctor/` first.

## Lane E: JS side, assets, parity harness

Files: `docs/research/qol-memory/**` and `plugins/qol-memory/assets/**` only.

1. `git mv` is a git command and forbidden for lanes: copy `concept-aliases.json` and `skills-glossary.json` to
   `plugins/qol-memory/assets/` and delete the originals with `rm`; the architect stages the rename.
2. Update every reference (`ask.mjs`, `skills.mjs`, `test-alias.mjs`, any doc that names the path) to
   `../../../plugins/qol-memory/assets/<file>` resolved from the script's own directory.
3. Fix ask.mjs line 153 `readFile` -> `readFileSync`.
4. Write `docs/research/qol-memory/parity.mjs`:
   - Inputs: `--bin PATH` (default `QOL_MEMORY_BIN` env, then `<repo>/target/debug/qol-memory`), `--store PATH`
     (default: the shared resolver), `--questions a.json,b.json` (default the three eval files), `--limit N`.
   - For each question (`query` field): run `node ask.mjs <q> --brief --no-log` and `<bin> --json ask <q> --brief --no-log`
     with the same env, parse both, deep-compare (absent key != null; numbers exact; arrays ordered). Then the same
     without `--brief` for the first 5 questions. Then two questions with `--exclude-session <session of the first
     unit in units.jsonl>`.
   - Pass 1: delete `idx-*.json` and `.meta` in the store, run JS first then Rust for each question. Pass 2: delete
     the caches again, run Rust first then JS. After each pass compare the `.meta` files the two sides wrote for the
     `user`, `pool`, and `notes` layers field by field (they must be identical).
   - Report every mismatch as a JSON pointer path with both values; exit 1 on any mismatch; print a one-line
     summary `parity: <passed>/<total> questions, <n> mismatches`.
   - Write the report to `<store>/eval/parity-<iso>/report.json` like eval.mjs does.
5. Add a `## Plugin` section to `docs/research/qol-memory.md` pointing at this spec (3 lines, no restatement).

## Non-goals (MVP)

- Daemon, watcher, ingestion, notes/decisions distillation in Rust, settings surface, gpui, dense/embedding rerank,
  query rewriting, `qol memory` CLI subcommand, moving the pi extension or the SessionStart hook.

## Lanes

- Round 1: `qm-scaffold` (Lane D): the whole crate skeleton compiling with `todo!()`-free stubs (return
  `Err(anyhow!("not implemented"))` or empty values), plugin.toml, Cargo.toml, build.rs, README, LICENSE,
  .gitignore, platform module copied from the template, main.rs with the manifest test adapted, cli.rs wired,
  every module file with the exact signatures above and a module-level doc comment naming its JS source file.
- Round 2, parallel: `qm-store` (A: src/store/**, src/retrieval_log/**), `qm-retrieval` (B: src/text/**,
  src/retrieval/**, src/aliases/**), `qm-skills` (C: src/skills/**), `qm-ask` (D: src/ask/**, src/cli.rs,
  src/doctor/**, README.md), `qm-parity` (E: JS side).
- Architect gate after round 2; correction rounds per lane as needed; squash delivery to main.
