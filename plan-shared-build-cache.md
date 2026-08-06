# Plan: single source of truth build cache across CI and release workflows

Status: proposal, measured 2026-08-06.
Deliver direct to main per qol-monorepo rules.
Read `qol-project:qol-cicd` and `qol-project:qol-arch-cicd` before editing anything.

## Goal

Every workflow that compiles the workspace should draw from one shared, deliberately managed cache pool per platform, so a release wave only compiles what actually changed.

Non-goals: making CI's own 13-22 minute check job faster (separate problem), byte-identical reproducible builds, moving off GitHub-hosted runners.

## Measured baseline (2026-08-06)

Cache quota: 9.44 GB used of the 10 GB repo limit.
Surviving entries are almost all `ci-*`; every `release-*`, `qol-tray-*`, and macOS candidate cache had already been evicted:

```
2.53 GB  v0-rust-ci-ubuntu-latest-...-23b0bf21      (main)
2.47 GB  v0-rust-ci-ubuntu-latest-...-a1a3eaee      (main)
1.91 GB  v0-rust-ci-macos-latest-...-a1a3eaee       (main)
1.62 GB  v0-rust-ci-macos-latest-...-23b0bf21       (main)
0.32 GB  v0-rust-ci-windows-sandbox-... (x2)        (main)
0.29 GB  v0-rust-release-candidate-x86_64-linux-... (main)
```

Job durations from recent successful runs:

| Job | Warm | Cold (evicted cache) |
|---|---|---|
| CI lint + test (ubuntu) | 20-22 min | n/a, its cache survives |
| CI lint + test (macos) | 13-15 min | n/a |
| Versioning plugin candidate, per target | 1.4-5.7 min | not directly observed, expect 10-20 min |
| Release Plugin build, per target | 2.8-5.3 min | not directly observed, expect 10-20 min |
| QoL Tray release build (macos) | 7.1 min (v3.48.5) | 19.5 min (v3.48.4, same job) |

The tray macOS pair is the measured cold-versus-warm delta: 12 minutes lost when the cache had been evicted between releases.

End-to-end release wave today: push, CI (13-22 min), Versioning candidates (2-6 min per job when lucky), dispatched release builds (3-5 min when lucky), so roughly 30 minutes on a lucky day and 45+ when the eviction lottery goes badly.
This plan removes the lottery and the variance, not the CI floor.

## Root causes

1. **13 cache namespaces for one dependency tree.** Shared keys today: `ci-ubuntu-latest`, `ci-macos-latest`, `ci-windows-sandbox`, `release-candidate-<target>` (3 targets), `release-<target>` (3 targets), `qol-tray-candidate-linux/macos`, `qol-tray-linux/macos`. Each holds largely the same compiled dependencies (gpui tree included).
2. **Working set far exceeds the 10 GB quota**, so LRU eviction constantly deletes the weekly-used release caches to make room for the daily-used CI caches.
3. **Flag mismatch blocks sharing anyway.** CI sets `RUSTFLAGS: -D warnings` (plus `-C link-arg=-fuse-ld=lld` on ubuntu). The `release.yml` build job and the `plugin_candidate` job in `plugin-version.yml` set **no RUSTFLAGS at all**. Different flags mean different cargo fingerprints and a different rust-cache key hash, so zero artifact reuse between CI and release-side builds. This is also a violation of the warning-parity hard rule in `qol-arch-cicd` (release builds are supposed to run `-D warnings`; only the tray jobs do).
4. **Layout mismatch.** CI builds `cargo build --release` with no `--target`; candidate and plugin release builds use explicit `--target <triple>`, which writes to a different directory tree with different fingerprints.
5. **Duplicated setup steps.** The apt package list, toolchain step, and rust-cache step are copy-pasted across `ci.yml`, `plugin-version.yml` (twice), `release.yml`, and `qol-tray-release.yml`, which is how the keys and flags drifted in the first place.

## Design

### Cache namespace contract

One namespace per (build family, platform).
Candidate and release builds of the same unit are byte-for-byte the same invocation (same script, same SHA, same lockfile), so they share a namespace.

| Workflow / job | shared-key today | shared-key after |
|---|---|---|
| ci.yml check (ubuntu, macos) | `ci-${{ matrix.os }}` | unchanged |
| ci.yml process-windows | `ci-windows-sandbox` | unchanged |
| plugin-version.yml plugin_candidate | `release-candidate-<target>` | `plugin-release-<target>` |
| release.yml build | `release-<target>` | `plugin-release-<target>` |
| plugin-version.yml qol_tray_linux_candidate | `qol-tray-candidate-linux` | `qol-tray-linux` |
| qol-tray-release.yml build_linux_deb | `qol-tray-linux` | `qol-tray-linux` |
| plugin-version.yml qol_tray_macos_candidate | `qol-tray-candidate-macos` | `qol-tray-macos` |
| qol-tray-release.yml build_macos | `qol-tray-macos` | `qol-tray-macos` |

Result: 13 namespaces become 8 (3 ci + 3 plugin-release targets + 2 tray).
The candidate job saves the cache minutes before the dispatched release job restores the exact same key (same lockfile, same env), so release builds become reliably warm even under quota pressure, because the entry is the newest in the pool.

Why the key match is exact: rust-cache's key is shared-key + runner + rustc version + env hash (RUST*/CARGO* vars) + lockfile hash.
Candidate and release jobs run the same SHA (the bump commit the tag points at), same runner image, same toolchain, and, after this plan, the same RUSTFLAGS.

### Flag parity contract

Every release-profile build job in every workflow sets the same RUSTFLAGS:

- ubuntu runners: `-D warnings -C link-arg=-fuse-ld=lld`
- macos and windows runners: `-D warnings`

This restores the `qol-arch-cicd` warning-parity rule for plugin builds and makes fingerprints compatible across jobs.
Use the existing ci.yml conditional as the pattern: `RUSTFLAGS: -D warnings ${{ matrix.os == 'ubuntu-latest' && '-C link-arg=-fuse-ld=lld' || '' }}` (the env var override drops `.cargo/config.toml`'s target rustflags, so lld must be restated, see the comment in ci.yml).

Safety property: candidates build before tags are created, and `tag_and_dispatch` requires candidate success, so a new `-D warnings` failure blocks the wave cleanly before anything is tagged or published.
That is the parity contract working, not a regression.

### Save policy

`save-if: ${{ github.ref == 'refs/heads/main' }}` on every rust-cache use.
ci.yml already does this; candidates (workflow_run context is main) and dispatched releases (`gh workflow run --ref main`) qualify; manual re-dispatch of an old tag on the tag ref does not, which stops dead caches from being saved on tag refs.

### Deterministic pruning replaces the LRU lottery

A new `.github/scripts/cache_prune.py`, run from `release-prune.yml` (the existing scheduled cleaner, which already owns janitorial work), enforces:

- Group caches by namespace: the key with the trailing 8-hex lockfile hash stripped (observed format `v0-rust-<shared-key>-<runner>-<envhash>-<lockhash>`).
- Keep the newest 2 entries per namespace, delete the rest.
- Also delete any entry not accessed for 14 days.

Delete via `gh api -X DELETE /repos/{owner}/{repo}/actions/caches?key=...` or by id; the job needs `actions: write`.
Ship a `--dry-run` flag and unit tests with fixture key lists (script changes require matching tests per `qol-cicd`; ci.yml's plan job runs them).

## Implementation phases

### Phase 1: unify keys, flags, and save policy (config only, one commit)

1. `plugin-version.yml` plugin_candidate: shared-key to `plugin-release-${{ matrix.target }}`, add the RUSTFLAGS env (conditional lld form), add save-if.
2. `release.yml` build: shared-key to `plugin-release-${{ matrix.target }}`, add the same RUSTFLAGS env, add save-if.
3. `plugin-version.yml` qol_tray_*_candidate: shared-keys to `qol-tray-linux` / `qol-tray-macos`, add save-if (RUSTFLAGS already correct).
4. `qol-tray-release.yml` both build jobs: add save-if (keys already `qol-tray-linux/macos`, RUSTFLAGS already correct).
5. After merge, delete the now-orphaned old-key caches once: `gh cache delete <key>` for every `release-candidate-*`, `release-*`, `qol-tray-candidate-*` entry still listed.

Do keys and flags in the same commit: each changes the cache key, so batching costs one deliberate cold wave instead of two.

Acceptance (next release wave):
- The candidate job log's rust-cache step shows a save, and the dispatched release job's rust-cache step shows `Restored from cache key` with the identical key string.
- No release-side build job exceeds ~7 minutes in a normal wave.
- `gh cache list` total drops below ~8.5 GB within a week.

### Phase 1b (recommended, small): `--locked` on release builds

Add `--locked` to the cargo commands in `release_candidate.py` `build_commands` (plugin build, tray macos builds; verify `cargo deb` forwards `--locked` before adding it there).
This turns manifest-versus-lockfile drift into a hard build error at release time; exactly this class of drift (pointz `Cargo.toml` 1.22.2 versus `plugin.toml` 1.23.0) broke the Versioning workflow on 2026-08-05.
Update `.github/scripts/tests/` to match.

### Phase 2: single source of truth for the duplicated steps

1. Create a repo-local composite action `.github/actions/rust-setup/action.yml` with inputs `cache-key` (required), `targets` (optional), and steps: dtolnay/rust-toolchain (SHA-pinned, same SHA as today), Swatinem/rust-cache (SHA-pinned, shared-key from input, save-if main), and the Linux apt dependency install (the canonical package list, one copy, guarded by `runner.os == 'Linux'`).
2. Replace the copy-pasted trio in `ci.yml` check, `plugin-version.yml` (three jobs), `release.yml` build, and `qol-tray-release.yml` (two jobs) with `uses: ./.github/actions/rust-setup`.
3. RUSTFLAGS stays declared per job in the workflow files (visible parity), values per the flag contract above.
4. Add `cache_prune.py` plus tests, wire a prune job into `release-prune.yml`.
5. Update the `qol-project:qol-cicd` skill in the qol-skills repo in the same delivery: document the composite action as the owner of shared setup steps, the namespace contract table, and the prune policy. Follow `marketplace-publishing` (bump plugin.json version, push the marketplace clone) and `standards-evolution` (encode the standard with the change, not after).

Scope note: the `qol-cicd-infra` hook forbids a separate workflow repository and reusable-workflow indirection.
A repo-local composite action is neither; it factors setup steps inside the owning repo and keeps the single-checkout workspace shape.
Do not move any behavior logic (build commands, verify gates) into the action; setup only.

Acceptance: `git grep -l 'libayatana' .github/workflows` returns nothing (list lives only in the action); all workflows still pass the YAML and script-test smoke checks below.

### Phase 3 (decision-gated): let release builds reuse CI's compiles

Only attempt after Phase 1 has a week of green waves, and only if candidate cold starts after lockfile changes still hurt.

Gate experiment first, locally on Linux:

```bash
cargo build --release --target x86_64-unknown-linux-gnu --workspace <same excludes and feature flags CI full mode emits>
cargo build --release --target x86_64-unknown-linux-gnu -p plugin-cli-sessions <its plugin.toml build features>
```

Count `Compiling` lines in the second command.
Near zero (the plugin crate itself is fine): feature unification is compatible, proceed.
Dozens of dependency recompiles: workspace-level feature unification diverges from single-plugin builds, stop here and document the finding in this file.

If proceeding: add `--target <host triple>` to CI's release-profile build step (ubuntu: `x86_64-unknown-linux-gnu`, macos: `aarch64-apple-darwin`) so CI's release tree lands in the same per-triple layout the candidates use, then point candidates at the ci namespace or vice versa.
Clippy and tests are unaffected (check and debug profiles, separate trees).
Note `x86_64-apple-darwin` is only ever built by candidate/release jobs (CI has no cross build), so its namespace stays candidate-fed.

## Considered and deferred, with the number that killed each

- **Artifact promotion** (candidates upload binaries, release.yml publishes without rebuilding): saves only the 3-5 warm minutes per release job that Phase 1 already secures, and adds a second delivery path plus artifact-expiry handling for manual re-releases of old tags (the immutable-releases re-dispatch flow must keep working). Revisit only if release-side latency still matters after Phase 1.
- **sccache with the GHA backend**: same 10 GB quota underneath, equally sensitive to the RUSTFLAGS mismatch, more moving parts. Fixing keys and flags first is strictly better; sccache remains an option layered on top later.
- **rust-toolchain.toml pin**: today `dtolnay/rust-toolchain@stable` floats, so a stable release day cold-starts every namespace at once (rustc version is part of every cache key). A pin makes keys deterministic, but also forces every local dev build onto the pinned toolchain and needs a manual bump chore. Decide separately; not required for this plan.
- **Self-hosted runner with a persistent disk**: removes the quota problem entirely, but is an operational and security posture change out of proportion to a variance problem.

## Verification checklist for the implementing agent

```bash
ruby -e 'require "yaml"; Dir[".github/workflows/*.yml"].each { |f| YAML.load_file(f) }; puts "ok"'
python3 -m unittest discover -s .github/scripts/tests -p 'test_*.py'
gh cache list --limit 100 --json key,sizeInBytes,ref   # before and after, record totals here
# after the first post-merge wave:
gh run list --limit 10
gh run view <versioning-run-id> --json jobs --jq '.jobs[] | "\(((.completedAt|fromdate)-(.startedAt|fromdate))/60) min \(.name)"'
# candidate and release job logs: confirm identical "Restored from cache key" strings
```

## Risks and rollback

- One deliberate cold wave when the keys and flags land (10-20 minutes per build job, once per namespace). Land Phase 1 right after a release wave, not mid-wave.
- `-D warnings` on plugin candidates can newly block a wave if the floating stable toolchain introduces a lint on unchanged code; the block happens pre-tag, the fix is a normal lint fix commit. This is the documented parity contract.
- A prune-script bug could delete a hot cache; worst case is one cold wave. Mitigate with `--dry-run` in the first deployed run and fixture-based tests.
- Rollback for any phase is a plain revert; caches repopulate on the next wave; nothing user-facing is touched.

## Constraints the implementing agent must respect

- Platform matrices stay derived from `plugin.toml` `platforms`; never hardcode runner lists (`qol-arch-cicd`).
- Third-party actions stay SHA-pinned, including inside the composite action.
- Any `.github/scripts/` change ships with tests in `.github/scripts/tests/`.
- No separate workflow repository, no reusable-workflow caller; the composite action is repo-local setup only (`qol-cicd-infra`).
- Commit style: conventional, type `ci` with no scope (the commit-msg hook rejects `ci` as a scope; `ci: ...` passes). No AI attribution lines, ever.
- Skill updates land with the change, not after (`standards-evolution`), including the marketplace version bump and push for qol-skills (`marketplace-publishing`).
- Direct to main, no PR unless asked; commit locally, push only when asked.
