# Sessions verification - qol check phase one

The sessions verification plan extends the existing `qol check` command instead of adding a second runner.
Phase one owns the machine evidence the architect needs to accept a round: an explicit base, exact owned-file formatting, source fingerprints that detect stale results, target-directory serialization, and a structured report.
The canonical skill policy lives in qol-skills commit 27f0fd7.
This note describes the phase-one implementation and the limits it deliberately carries.

## Command contract

`qol check` keeps the no-flag worktree behavior, `--staged`, and `--lint`.
`--base REV` resolves the revision to one commit with `git rev-parse --verify REV^{commit}` and feeds that commit to the existing affected planner as `BASE_SHA`.
The default base stays `origin/main` merge-base or `HEAD^` for worktree and staged checks, and `HEAD` for lint.
An explicit `--base HEAD` is used as requested, so the planner compares the worktree against HEAD and never widens to unrelated historical commits.
`--report PATH` publishes the authoritative report to the requested location after the run directory report is written.
Relative report paths resolve from the current working directory.
A missing or dash-prefixed `--report` value is rejected, so `--report --lint` cannot swallow the mode flag.
`--format-owned PATH` is repeatable and accepted only by full worktree checks.
`--staged --format-owned` and `--lint --format-owned` are rejected before any write.

## Owned-file formatting

Every requested path is validated before the formatter runs: repository-relative, no `..` components, no glob metacharacters, existing regular Rust source file, and a canonical path inside the repository root.
Duplicate spellings of the same file collapse to one planned file.
The owning crate or workspace `edition` is discovered by walking up from the file to the repository root.
The nearest `rustfmt.toml` or `.rustfmt.toml` on the ancestor path is discovered by explicit traversal, and its directory is passed as `--config-path` only when a config exists, so a no-config repository keeps rustfmt defaults.
The formatter prerequisite is probed once with `rustfmt --version` before the first write.
Each file is sent to `rustfmt --edition <edition> --emit stdout` over stdin, so the formatter never opens or writes a module graph.
The rustfmt child runs under the existing containment and cancellation owner with stdin attached and stdout captured; cancellation stops the child tree and prevents later files from being written.
Immediately before each rewrite the current bytes, file identity, and canonical path are compared against the input that was formatted; a concurrent edit fails the run without overwriting.
The formatted bytes are written back atomically only when they differ, and the before and after SHA-256 hashes are recorded.
Completed rewrites are retained in the report even when a later file fails, together with the failing file.
No source rewrite, staging, or `cargo fmt` fallback happens on failure.

## Report destination

A `--report` destination is validated before the run starts and again on parse-error publication.
Tracked source paths, symlinks, directories, and existing files that are not a prior `qol-check` report are refused, and a path whose symlinked ancestor resolves differently from its lexical location is refused.
An invalid destination never overwrites source, including when argument parsing already failed.
Caller-chosen report paths are not excluded from source identity: publication happens after the fingerprint is recaptured, so the report write cannot mask a source change.

## Source identity

Before checks run, the worktree is fingerprinted with SHA-256 over the HEAD commit, the index tree, and every tracked, untracked, deleted, or mode-changed path reported by `git status --porcelain=v2 -z --untracked-files=all --no-renames`.
Ignored build outputs are excluded by construction.
Symlinks are hashed by their target string and never followed.
Only the run directory and an approved effective target directory are excluded, and those exclusions apply only to untracked artifact records; tracked records and tracked bytes are always hashed, so a broad exclusion cannot hide a source edit.
An unresolved merge conflict, a rename record, or an index carrying skip-worktree or assume-unchanged flags is an explicit failure, never a verified pass.
The fingerprint is recaptured after checks; a mismatch marks the report `stale` and fails the run with a nonzero exit.
A `--lint` report always declares `lint-only` verification and never claims a full check.

## Target serialization

The effective target directory is resolved canonically once per run for locking and fingerprint exclusion.
Full worktree checks honor an absolute or relative `CARGO_TARGET_DIR`; lint and staged checks use their private targets under the repository `target` directory.
The repository root as a target is refused.
An in-tree target that contains tracked files is refused.
An in-tree target is excluded only when it is the node-owned default `target` subtree or provably git-ignored; any other in-tree location is an explicit unsupported input, and external targets are used without an exclusion.
Lock acquisition is a single `try_lock` with an explicit contention message; there is no wait loop and no new daemon.
Cancellation is checked after acquisition and reported explicitly.

## Report

The run directory report is extended additively with requested and resolved base, verification scope, source fingerprints before and after, formatted files and hashes with the failing file, per-command argv and exit code, publication state, and explicit `pending`, `verified`, `failed`, `stale`, `cancelled`, and `error` states.
Publication begins with a durable `pending` record that carries the requested path and no published path; the published state is written only after the external atomic rename reports success, and the final internal report is rewritten from the same bytes, so a completed run leaves identical internal and external reports.
A failed publication is recorded in the run-directory report, clears the published path, and rewrites the final report so it cannot claim `verified`.
Recoverable intermediate states are explicit: an interrupted attempt leaves the authoritative report pending with no external copy, and an interrupted final rewrite leaves the authoritative report pending while the external copy exists; neither state claims successful publication.
A parse failure publishes a validation report to the requested path whenever a repository root is resolvable and the destination is valid.

## Phase-one limits

Formatter ownership does not prove that other writers are idle.
Phase one is architect-invoked after lane fan-in, and the fingerprint only detects drift during the run.
The pre-write comparison narrows the formatter clobber window but does not remove the final rename race against an external writer.
Publication uses single-file atomic renames and makes no cross-file atomicity claim; the pending record is the durable recovery point for an interrupted publication.
Phase two will enforce round quiescence before automatic invocation and wire the typed verification request into sessions round metadata.
No automatic acceptance, retry loop, arbitrary shell hook, or budget widening is part of this phase.
