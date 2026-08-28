# Phase 4 spec: harness bridge consolidation (qol-skills)

Status: architect contract for the `bridge-plugin` and `bridge-project` lanes. Source plan: `docs/research/qol-memory/interface-plan.md` section 6. Repository: the qol-skills worktree at `/media/kmrh47/WD_SN850X/Git/worktrees/qm-bridge/qol-skills` (branch `qm-bridge`, based on `origin/main` 307eb1b). The stale clone at `Git/qol-skills` is not used. Facts below come from the 2026-08-28 scout of that worktree.

Rules for every lane: edit only the paths in your ownership row; never run build, test, lint, format, sync scripts or git commands; add no code comments; never use the em-dash character anywhere. Report changed files and lines plus conscious deviations, nothing else.

## 1. Goal

- A new marketplace plugin `qol-memory` gives Claude Code the tray MCP endpoint (tools `qol-memory__ask`, `qol-memory__status`, `qol-memory__capture`) with no memory-specific node process, the session-start continue block, and the pi live-capture extension.
- The memory hook, script, test and pi extension leave `qol-project`.
- Every harness-side call goes through the tray HTTP API (`http://127.0.0.1:42700`, header `x-qol-token`) instead of parsing the store or spawning node scripts: `POST /api/plugins/qol-memory/queries/continue` (query input route, lands with phase 3) and `POST /api/plugins/qol-memory/actions/capture`.
- Codex, kimi and pi reach the same tools through `qol mcp configure <harness>` (tools/qol-cli/src/commands/mcp/configure.rs), which is the only way to carry the per-host token; the plugin ships no static token.

Decisions taken by the architect, deviating from the plan text:

- The Claude MCP entry is declared inline in `.claude-plugin/plugin.json` (`mcpServers` object) rather than in `.mcp.json`, because `scripts/sync-plugin-manifests.cjs` points the Codex manifest at `.mcp.json` whenever it exists and Codex cannot evaluate `headersHelper`.
- The continue hook no longer parses `units.jsonl`; it asks the daemon through the tray. When the tray is down it prints nothing and exits 0.
- The `qol_memory_retrieve` pi tool is removed; pi gets the MCP tools through `qol mcp configure pi` (already writes `~/.pi/agent/mcp.json` with `headers: {"x-qol-token": "!qol mcp token"}`).

## 2. Ownership

| Lane | Owned paths (all under the qm-bridge worktree) |
|---|---|
| `bridge-plugin` | `plugins/qol-memory/.claude-plugin/plugin.json`, `plugins/qol-memory/hooks/hooks.json`, `plugins/qol-memory/bin/qol-tray-http.cjs`, `plugins/qol-memory/bin/inject-qol-memory-continue.cjs`, `plugins/qol-memory/.pi/extensions/qol-memory-tool.ts`, `plugins/qol-memory/skills/qol-memory/SKILL.md`, `plugins/qol-memory/test/qol-tray-http.test.cjs`, `plugins/qol-memory/test/inject-qol-memory-continue.test.cjs` |
| `bridge-project` | `plugins/qol-project/hooks/hooks.json`, `plugins/qol-project/.claude-plugin/plugin.json`, deletion of `plugins/qol-project/bin/inject-qol-memory-continue.cjs`, `plugins/qol-project/.pi/extensions/qol-memory-tool.ts`, `plugins/qol-project/test/inject-qol-memory-continue.test.cjs` |

Generated files (`.codex-plugin`, `.kimi-plugin`, `.pi-plugin` manifests, `.pi/extensions/hooks.ts`, the three marketplace files, `kimi.plugin.json`, root `package.json`) are produced by the architect with `node scripts/sync-plugin-manifests.cjs` at the gate; lanes never touch them.

## 3. Lane `bridge-plugin`

### 3.1 `.claude-plugin/plugin.json`

```json
{
  "name": "qol-memory",
  "description": "Long-context memory for agent sessions: ask settled facts from prior sessions, capture new ones, and get the units that landed since your last session in this directory.",
  "version": "0.1.0",
  "author": {
    "name": "KMRH47"
  },
  "mcpServers": {
    "qol": {
      "type": "http",
      "url": "http://127.0.0.1:42700/api/mcp",
      "headersHelper": "qol mcp headers"
    }
  }
}
```

Read `scripts/sync-plugin-manifests.cjs` (SHARED_FIELDS at line 6, codexManifest around 359-377, marketplace writers) and confirm in your report that an extra `mcpServers` key in this file is neither copied to the other manifests nor rejected by `--check`. If the script would copy or reject it, say so and keep the file as written.

### 3.2 `hooks/hooks.json`

Same shape as `plugins/qol-sessions/hooks/hooks.json`: one `SessionStart` entry with matcher `.*` whose command is the identical inline `node -e '...'` cache-resolver wrapper used by qol-project and qol-sessions, with trailing args `qol-memory bin/inject-qol-memory-continue.cjs`. Copy the wrapper verbatim from `plugins/qol-project/hooks/hooks.json` line 37; only the two trailing args differ. `description`: `Session-start recall of the memory units that landed since the last session in this directory.`

### 3.3 `bin/qol-tray-http.cjs`

```js
module.exports = { baseUrl, readToken, postJson };
```

- `baseUrl()`: `process.env.QOL_TRAY_BASE_URL` when non-empty, else `http://127.0.0.1:42700`.
- `readToken()`: `process.env.QOL_TRAY_HTTP_TOKEN` when non-empty; else the trimmed contents of the first existing file among `$XDG_CONFIG_HOME/qol-tray/.http-token`, `~/.config/qol-tray/.http-token`, `~/Library/Application Support/qol-tray/.http-token`; else `null`.
- `postJson(path, body, timeoutMs)`: resolves `{ status, body }` where `body` is the parsed JSON (or `null` when unparseable) after a POST to `baseUrl() + path` with headers `content-type: application/json` and `x-qol-token: <token>`; rejects on a missing token, a connection error, or the timeout. Uses `node:http` only.

### 3.4 `bin/inject-qol-memory-continue.cjs`

- Reads stdin JSON; needs `cwd` and `session_id` as non-empty strings, else exits 0 with no output.
- Exits 0 with no output when `QOL_MEMORY_CONTINUE_DISABLE=1`.
- `postJson("/api/plugins/qol-memory/queries/continue", { cwd, session: session_id }, 1500)`.
- When `status` is 2xx and `body.stage === "injected"` and `body.block` is a non-empty string, prints `{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext": block}}` as one line; otherwise prints nothing. Every failure path exits 0 silently.

### 3.5 `.pi/extensions/qol-memory-tool.ts`

Start from `plugins/qol-project/.pi/extensions/qol-memory-tool.ts` (180 lines) and keep: `redact()`, the store path resolution, the `message_end` user-unit capture, the `session_compact` compaction unit plus the debounced `decisions.mjs --live` distill trigger, and the 12 hour catch-all distill guard. Change the two unit writes to go through the tray first: `postJson("/api/plugins/qol-memory/actions/capture", { unit }, 2000)` loaded with `createRequire(import.meta.url)` from `../../bin/qol-tray-http.cjs`; on any rejection or non-2xx status fall back to the existing direct `appendFileSync` to `units.jsonl`. Remove `pi.registerTool("qol_memory_retrieve")`, the `ask.mjs` spawn, the `/tmp/qol-memory-tool-calls.log` writes and every helper only they used. Keep `QOL_MEMORY_LIVE_CAPTURE_DISABLE=1` honoured.

### 3.6 `skills/qol-memory/SKILL.md`

Frontmatter `name: qol-memory` and a one-sentence `description` starting with `Use when`. Body, timeless prose only (no counts, versions, dates, status words; `scripts/check-skill-invariants.cjs` lints it): when to call `qol-memory__ask` (before re-deriving a fact about paths, decisions, commits or prior fixes in this workspace), what `qol-memory__capture` expects (one self-contained sentence with the identifiers a later reader needs, and the absolute project directory as `cwd`), that `qol-memory__status` reports store health, that the session-start block lists units landed since the last session in the directory, and that other harnesses connect with `qol mcp configure <harness>`. Under 60 lines.

### 3.7 Tests (`node:test`, `node:assert`, same style as `plugins/qol-sessions/test/*.test.cjs`)

- `test/qol-tray-http.test.cjs`: `readToken` prefers the env var; `postJson` sends the header and JSON body to a local `http.createServer` on an ephemeral port and resolves the parsed body; a closed port rejects.
- `test/inject-qol-memory-continue.test.cjs`: run the script with `spawnSync` and env `QOL_TRAY_BASE_URL` pointing at a local server plus `QOL_TRAY_HTTP_TOKEN=t`: the server answering `{"stage":"injected","block":"[qol-memory continue] 2 unit(s) landed"}` yields the hook JSON with that `additionalContext`; `{"stage":"quiet"}` yields empty stdout; a closed port yields empty stdout and exit 0; `QOL_MEMORY_CONTINUE_DISABLE=1` yields empty stdout without contacting the server.

## 4. Lane `bridge-project`

- `hooks/hooks.json`: delete the `SessionStart` entry with matcher `.*` that runs `bin/inject-qol-memory-continue.cjs` (line 37); keep the `startup|resume|clear` entry and everything else byte for byte.
- Delete `bin/inject-qol-memory-continue.cjs`, `.pi/extensions/qol-memory-tool.ts`, `test/inject-qol-memory-continue.test.cjs`.
- `.claude-plugin/plugin.json`: `version` `0.8.31` -> `0.8.32`; trim the description only if it mentions memory (it does not today; leave it).
- Report every remaining `memory` match under `plugins/qol-project` (rg) so the architect can confirm only RAM-related and skill-prose matches remain.

## 5. Gate and acceptance (architect)

1. In the worktree: `node scripts/sync-plugin-manifests.cjs`, then `node scripts/sync-plugin-manifests.cjs --check`, `node scripts/check-skill-invariants.cjs --check`, `node --test test/*.test.cjs plugins/*/test/*.test.cjs`.
2. Live hook against the tray after phase 3 is recompiled: `printf '{"cwd":"/media/kmrh47/WD_SN850X/Git/qol-monorepo","session_id":"spec-check"}' | node plugins/qol-memory/bin/inject-qol-memory-continue.cjs` prints the continue JSON or nothing, exit 0.
3. `qol mcp configure pi` on this host; `~/.pi/agent/mcp.json` carries the `qol` entry; `tools/list` over curl still lists the three tools.
4. One commit on `qm-bridge` (`feat(qol-memory): move the memory bridge into its own plugin`) including the regenerated manifests and marketplace files; no push. Claude Code loads marketplace plugins from pushed commits, so the fresh-session tool listing is verified after the user's push.

## 6. Out of scope

- Static tokens in any shipped file, a `qol-memory` README, changes to `qol-sessions`, and any change to the pi hook bridge generator.
