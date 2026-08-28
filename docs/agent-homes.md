# Agent homes

An agent home is one harness instance's home directory: where a Claude Code,
codex, kimi, or pi installation keeps its config and transcripts.
`libs/qol-agent-homes` is the single source of truth for the home rules, and one
host file declares which homes this machine has.
Everything that needs an agent home reads the registry instead of copying env
var rules into each consumer.

## Registry file

The registry lives at `~/.config/qol-tray/agents.toml`, the qol-config
`config_dir()` joined with `agents.toml`.
It is one machine-level file, never install-scoped; when the config directory
cannot be resolved the registry holds only the implicit defaults.
A missing file means no declared homes.
A file that exists but cannot be read or parsed also means no declared homes,
and `qol agents list` reports the error.

```toml
[[home]]
harness = "claude"
path = "~/.claude-work"
shared = false
default = true
```

- `harness`: one of `claude`, `codex`, `kimi`, `pi`; unknown names are skipped.
- `path`: the home directory; a leading `~` expands against the user home, a
  relative path joins onto the user home, trailing separators are dropped, and
  blank paths are skipped.
- `shared`: optional, default `false`; when `true`, every caller may read this
  home's memory.
- `default`: optional, default `false`; the home assumed for this harness when a
  request names none.
- A home's identity is its normalized absolute path (the id); ids compare as
  strings everywhere.

## Implicit defaults

Every harness always has its builtin default home in the registry, marked
implicit.
A declared entry whose id equals the builtin path takes the builtin slot, its
shared flag wins, and it is the harness default unless another declared entry
of that harness carries `default = true`; when several declared entries claim
default, the first wins and the others are cleared.
Implicit pi is `shared = true`; the other implicit entries are `shared = false`.
Declared order is kept, and implicit entries come last.

## Environment variables

| Harness | Home env var | Builtin default home |
|---|---|---|
| claude | `CLAUDE_CONFIG_DIR` | `~/.claude` |
| codex | `CODEX_HOME` | `~/.codex` |
| kimi | `KIMI_CODE_HOME` | `~/.kimi-code` |
| pi | `PI_CODING_AGENT_DIR` | `~/.pi/agent` |

The env var wins when it is non-empty: a value matching a registered home
resolves to that home, and any other value resolves to an ad hoc home that is
not shared, not default, and unregistered.
An undeclared env home is never ingested and sees only shared units until it is
declared with `qol agents add`.
`qol agents list` shows it as an extra `unregistered` row so the gap is visible.

## CLI verbs

`qol agents` inspects and edits the registry.

```
qol agents list [--json]
qol agents current <claude|codex|kimi|pi> [--json]
qol agents add <claude|codex|kimi|pi> <path> [--shared] [--default]
qol agents remove <path>
```

- `list` prints one tab-separated row per home: `harness`, `id`, `shared` or
  `-`, `default` or `-`, then `declared`, `implicit`, or `unregistered`; each
  harness whose env home is set but not registered adds one extra
  `unregistered` row; `--json` prints `{"homes": [...]}`, the same shape as
  `GET /api/agents`, plus an `"error"` key when the registry file could not be
  read or parsed.
- `current` prints the id the harness resolves to right now; this is what
  scripts call; `--json` prints the `AgentHome`.
- `add` appends or updates the `[[home]]` entry, creating the file; existing
  comments and formatting survive; `--default` clears `default` on that
  harness's other entries; an update rewrites both flags, so pass `--shared`
  and `--default` again when updating an entry that carries them; the
  confirmation is followed by the resulting row.
- `remove` deletes every entry whose normalized path matches, regardless of
  harness, lists each removed harness, and errors when nothing matched.

## Recipes

### Work PC with two claude instances

When the work instance owns `~/.claude`, declare the personal one:

```
qol agents add claude ~/.claude-personal
```

Then run the personal shell with `CLAUDE_CONFIG_DIR=~/.claude-personal`.
When the personal instance owns `~/.claude` instead, declare the work one with
`qol agents add claude ~/.claude-work` and set the env var in the work shell.

### Making pi private

Implicit pi is shared because pi lanes are subordinate to the session that
spawned them, and pi has no per-instance home convention to split personal from
work lanes.
To make pi private, declare it without `--shared`:

```
qol agents add pi ~/.pi/agent
```

### Re-homing codex or kimi

`qol mcp configure codex` and `qol mcp configure kimi` bake the home id into
their config as a static header, so re-run the command after re-homing that
harness.
Claude and pi resolve their home live on every call and need no re-run.

## Rules worth knowing

- Paths are identity: a unit belongs to the home id that produced it, so moving
  a home orphans its units until the new path is re-added with
  `qol agents add`.
- Fail closed on MCP: on a machine with more than one home for a harness, an
  MCP call without the `x-qol-agent-home` header is refused instead of
  guessing; with a single home per harness the plugin resolves its default as
  today.
- Fail closed in the hook: the qol-skills continue hook skips the recall when
  `qol agents current claude` fails, rather than answering from the wrong home.
- Sessions limitation: the sessions builtins resolve one claude home per
  observing process (the tray's or watcher's own env), while memory ingests
  every registered home.
- Ingest roots resolve once at daemon start: after `qol agents add` or
  `qol agents remove`, and after a declared home's transcript directory is
  first created, restart qol-tray before that home's transcripts are
  ingested; `qol-memory doctor` lists the roots the registry holds, not the
  roots the running daemon watches.
- Upgrade qol-tray and the qol-memory plugin together: an ask output that
  lacks the `agent_home` field means an old build is answering and the
  partition is not enforced for that call.
- Upgrade note: nothing rewrites the units recorded before agent homes
  existed; they resolve live to the current default home of their source, pi
  units to the default pi home and every other legacy unit to the default
  claude home, so moving the default with `--default` re-parents them at the
  next query. Codex and kimi transcripts are not ingested until parsers
  exist.

## Consumers

- `qol-memory` tags every memory unit with the agent home id that produced it
  and partitions recall by caller, so a personal Claude Code home never reads
  work transcripts and the other way round.
- `qol-terminal-sessions` derives each harness builtin's home from the registry,
  so `CLAUDE_CONFIG_DIR` and friends resolve during session discovery.
- `qol mcp headers` and `qol mcp configure` put the caller's home id in the
  `x-qol-agent-home` header.
- The qol-skills bridge scripts resolve the caller with
  `qol agents current <harness>` instead of duplicating env var rules in
  JavaScript.
