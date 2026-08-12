# Diff Viewer Architecture

The diff viewer is a hotkey-summoned GPUI window that renders repository
change as motion and heat.
Wave 0 lands the architecture as compiling code: the change model, the git
facts layer, the TTL cache, the CodeSurface contract, and this system contract.
Wave 0 does not build the viewer UI, the diff engine, or the lexer.

## Data flow

The pipeline is: watcher, git facts cache, diff model, CodeSurface, user.
A watcher observes the active repo working tree and fires change signals.
The git facts layer reads local git state through the `git` CLI and stores
facts in the TTL cache.
The diff engine (wave 1) converts cached facts into the qol-diff change model.
CodeSurface (wave 2) renders the change model as a virtualized text surface.
The user drives the surface with keyboard, scroll, and the summon hotkey.
qol-git owns local-read facts only: head sha, branch, porcelain status,
numstat, raw patch text, and log entries.
qol-diff owns the change model and the failure states; it has no git
dependency and no gpui dependency.
The viewer crate owns CodeSurface and every other viewer widget (arch-5);
nothing moves to qol-gpui or qol-theme until a second consumer exists.

## Threading

The gpui main thread owns all model state and renders.
The watcher runs in-process on its own thread (accepted risk: watching stops
when the panel closes).
`git` CLI spawns happen on a background thread, never on the gpui main thread.
Results cross the thread boundary through a channel into the main thread.
Cancellation uses a generation counter: each refresh bumps the generation,
and stale results arriving with an old generation are dropped.
The background thread checks the generation before and after each spawn.
A long diff render never blocks the hotkey response because the porcelain
fast path and the full diff are separate stages.

## Cache invalidation

The git facts cache is keyed by repo path plus head sha plus index mtime.
A cache entry is valid only when repo, head sha, and index mtime all match
the live values.
The porcelain fast path runs on every refresh signal and detects head or
index changes.
The TTL is a backstop, not the primary invalidation signal.
qol-cache uses a sliding TTL: a hot key stays warm, stale keys expire on
read, and expired entries are pruned on insert.
The full diff is cached per repo and range and is invalidated by the same
key contract, so an amend or rebase churns the filmstrip correctly.

## Latency budget

Summon-to-first-paint must stay under 500 ms.
The measured fast path is `git status --porcelain` at 8 ms on this repo, so
in principle the budget is met by the porcelain fast path plus cached facts.
In practice the first frame does not yet contain the fast-path file list:
measured in a guest at monorepo scale (8k directories under the repo), the
first facts landed 81-96 ms after the first render because the main thread
was busy registering the recursive watch and the result poll ticks at 50 ms.
The first painted frame is therefore the empty state and the file list
appears a poll tick or two later (measured after the fix: +84 to +140 ms,
now gated by the cold git spawn and the poll phase instead of by tree
size); the full diff runs async and paints when ready.
A large-range full diff measured 89 to 97 ms and grows with repo size, which
is why it never blocks the first paint.
Watch registration is off the paint path: it runs on a background thread, so
open latency no longer scales with tree size (before the fix, first render
grew ~18-20 us per directory: 299 ms at 8.1k dirs and 380 ms at 12.2k dirs;
after the fix it stays 166-193 ms with target/ trees from 8k to 20k dirs
and a rejecting 20.5k-dir non-noise tree). Registration is filtered at
registration time (build-noise directories `target/`, `node_modules/`, and
`.git` are excluded before any watch is taken), so they consume neither
inotify watches nor walk time, and a per-root budget
(`pipeline::WATCH_BUDGET`, 20,000 directories) rejects registration above
the budget. A pathological tree therefore either registers within budget or
degrades deterministically to the 3 s backstop refresh instead of silently
exhausting the OS watch limit mid-walk.

## Failure model

Binary files, encoding failures, conflict markers, and empty diffs are
explicit qol-diff states, not crashes.
DiffError::Binary covers files git reports as binary.
DiffError::Encoding covers non-UTF-8 text content.
DiffError::Conflict covers unresolved merge markers in the patch.
DiffError::Other covers every remaining failure.
FileDiff::empty() is the no-change state and renders as an explicit empty
UI state.
Every failure state renders as its own visible UI state with a reason, per
the mission rule that failures stay visible.

## Lexer decision

Wave 2 ships a minimal hand-rolled per-language tokenizer.
Scope is strings, comments, and keywords; no tree-sitter, no syntect, no new
dependencies.
Token spans live in LineChange.token_spans as byte offsets with a heat level.
MVP heat comes from the engine's char-level diff of changed lines; the lexer
later refines and extends those spans. Structural diff and real grammars are
explicit stretch goals.

## Wave 0 spikes

Spike A, fonts: this host exposes 53 monospace fontconfig entries, with
DejaVu Sans Mono, Liberation Mono, Nimbus Mono PS, Noto Mono, Noto Sans Mono,
and Ubuntu Mono as the real families.
gpui 0.2.2 all_font_names() returns platform fonts plus the fallback stack
plus ".SystemUIFont", sorted and deduped.
On Linux gpui resolves a family by name through cosmic-text fontdb, so any
family present in fontconfig resolves.
resolve_font() falls back through a fixed stack and panics if nothing
resolves; ".SystemUIFont" maps to the platform system face and is the only
guaranteed face.
Chosen fallback chain: "Noto Sans Mono", then "DejaVu Sans Mono", then
"Liberation Mono", then ".SystemUIFont".
CodeSurface picks the chain at runtime by testing membership in
all_font_names() and never calls resolve_font() with a missing name.

Spike B, git transport: measured in this worktree, 2716 tracked files and
1029 Cargo.lock packages, kernel page cache warm (no root to drop caches).
`git status --porcelain` measured 8 ms across five runs on a clean tree and 9 ms on a dirty tree with seven changes.
`git diff --numstat HEAD~5` measured 19 ms cold-process and 11 ms warm.
`git diff HEAD~100` full patch measured 89 to 97 ms.
`git log -n 50` measured 2 ms.
Conclusion: the CLI transport meets the 500 ms budget with the porcelain
fast path, so git2 stays out of the viewer (arch-10); the diff engine parses
the raw patch text qol-git returns.

## Wave plan

Wave 0 (this commit): system contract, lexer decision, spikes, change model,
CodeSurface signature stubs, crate skeletons, workspace dependencies, and
lockfile, all by one agent (arch-6).
Wave 1 (4 parallel, one crate owner each): qol-cache, qol-git local-read
core, qol-diff engine plus change model, CodeSurface in the viewer crate.
Wave 2 (3 parallel): LinkScroll, OverviewMap plus Scrubber, lexer plus token
spans, all in the viewer crate.
Wave 3 (2 parallel): DiffView composition and the plugin shell with summon,
watcher, session click via qol-terminal-sessions, doctor, help, settings,
and trace targets.
Wave 4: per-wave review gate and guest-VM verification with a repo fixture.
Cleanup wave later: CommandRouter, RetainedWindows, qol-stream, and state
store unification, sequenced after the viewer pulls on them.
RetainedWindows, CommandRouter, and qol-stream are explicitly out of wave 1
(arch-3, arch-11).

## Contract discipline

Contracts are compilable Rust signature stubs, frozen in wave 0, amended
only through this document's amendment log.
A contract change is broadcast before any implementer builds against the
new shape.
diff_patch returns the raw unified diff text; parsing it into FileDiff is
the wave 1 engine's job.
qol-git stays local-read only: no auth, no write ops, no git2 (arch-9).

## Amendment log

2025-08-10: LineChange gains old_line_no and new_line_no (Option<u32>), set by the engine while parsing hunks, so the CodeSurface gutter can render old/new line numbers. Additive change.
2025-08-10: heat source corrected: engine char-level diff first (MVP), lexer refines later. No type change.
2025-08-10: qol_git::diff_patch gains a paths slice: diff_patch(repo, range, paths). The viewer's SelectFile no longer sends a precomposed "HEAD -- path" range string, which git cannot parse as a single argv element. Git-accurate shape; empty slice diffs the whole range.
2025-08-10: repo resolution: QOL_DIFF_REPO env override, else walk-up from launch cwd to the nearest .git. The tray spawns runtime actions with cwd = plugin dir, so the env override is the summon-time repo source until session-click integration lands.
2025-08-10: TokenSpan gains kind: TokenKind (Plain/String/Comment/Keyword, default Plain) so the lexer can drive syntax coloring; heat and kind coexist on one span. Additive change.
2025-08-10: MVP verification notes: the prepared desktop images ship without a git binary and are offline by design (-nic none), so guests cannot run git-dependent plugins end-to-end; guest round verified the dispatch route and surface states, the rendering pipeline was verified with a real repo on the host metal.
2026-08-12: live heat decays. qol-diff gains decayed_heat(HeatLevel, elapsed): Hot cools to Warm at 60 s and Warm cools to Cool at 300 s, both thresholds measured as the age since the last touch; Cool never warms. The pipeline keeps a path -> last-touch stamp map keyed by the watch signals it already receives; watch paths are normalized to repo-relative (they arrive absolute, numstat/select paths are relative, which also restores the watch-refetch match the refetch commit intended) and worktree diff production stamps too, while commit-range selects never stamp so history browsing cannot reset live heat. Diff results carry touched_at and the view re-renders decayed heat on a 1 s tick that never spawns git and notifies only when a heat transition lands. Only the worktree range decays; surface.rs color mapping is byte-identical, decay feeds it a different HeatLevel.
