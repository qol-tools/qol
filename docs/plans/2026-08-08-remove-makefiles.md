# Retire every Makefile

**Outcome: zero Makefiles in the repository.** All 11 are deleted, including
`apps/qol-tray/Makefile`. There is no keeper, no reduced Makefile, and no root Makefile introduced
to replace them. `make` stops being a way to build anything in this repo, and
`find . -name Makefile -not -path '*/target/*'` returns nothing.

`tools/qol-cli` already describes itself as "qol-tray dev orchestrator (replaces make)"
(`tools/qol-cli/Cargo.toml:5`). The 11 remaining Makefiles are the unfinished half of that
migration. Nothing in CI, `.githooks`, or the `qol` CLI invokes make; every caller is a human
following stale prose.

## Inventory

| File | Targets | Shape |
|---|---|---|
| `apps/qol-tray/Makefile` | 20 | the only substantial one |
| `plugins/template/Makefile` | 11 | the original the clones came from |
| `plugins/{controllers,qol-voice,removeapp}/Makefile` | 11 | exact template clone, `BINARY` renamed |
| `plugins/bluetooth/Makefile` | 12 | template clone plus `smoke-daemon`, and the only one with `TARGET_DIR = ../../target` |
| `plugins/{lights,os-themes}/Makefile` | 10 | template clone minus `ci-local` |
| `plugins/alt-tab/Makefile` | 4 | trimmed variant (`dev release test trace`) |
| `plugins/keyremap/Makefile` | 3 | `dev release test` |
| `plugins/ide-checkout/Makefile` | 2 | `dev release` |

`plugins/{cli-sessions,qol-shot,window-actions,...}` never had one, and they build fine. That is
the proof the plugin Makefiles are optional.

## What is already dead

- **Plugin `dev` / `release`, in 9 of 10 plugins.** Both run
  `install -m 755 target/debug/$(BINARY) ./$(BINARY).new` from the plugin directory. Post-monorepo
  there is no `plugins/*/target`; the binary lands in the workspace root `target/`.
  `ls -d plugins/*/target` returns nothing. These targets fail today. `plugins/bluetooth` is the
  single exception: someone patched it with `TARGET_DIR = ../../target`, so its staging still
  works, which is itself evidence the other nine went unnoticed.
- **Tray `release`.** Bumps `Cargo.toml`, commits `chore(release): v$NEW`, tags `v$NEW`, pushes.
  No plain `v*` tag exists in the repo. Real tray tags are `qol-tray-v*`
  (`.github/workflows/qol-tray-release.yml:13`) and the bump itself is done in CI by
  `plugin-version.yml:116`. Running `make release` would push a wrong tag onto `main`.
- **Tray `diagram`.** `apps/qol-tray/README.md:20` already tells the reader to open
  `diagram/Runtime Architecture Map.html` directly.
- **Plugin `fmt` / `lint` / `ci-local`.** `cargo fmt --all` and `cargo clippy --all-targets` inside
  a workspace member act on the whole workspace, so these silently do something other than what
  their name promises.

## Before and after

Every row was checked against the CLI source, not against `qol help` prose.

### `apps/qol-tray/Makefile`

| Target | Before | After | Status |
|---|---|---|---|
| `setup` | `bash scripts/dev-setup.sh` | nothing | deleted, see gap 1 |
| `$(DEV_HOOKS)` | same, as an order-only prerequisite of `dev` | nothing | deleted, see gap 1 |
| `build` | `lint` then `cargo build` | `qol build` | direct (`commands/build.rs:56`) |
| `check` | `lint` then `cargo check` | `qol check` | direct (`check/mod.rs:294,321`) |
| `test` | `lint` then `cargo test` | `qol check` | `check/mod.rs` runs fmt, clippy, cargo tests, UI tests, release-script tests |
| `run` | `lint` then `cargo run --bin qol-tray` | `qol dev` | direct |
| `dev` | `recompile-linked`, dev hooks, `fmt-check`, `pkill -x qol-tray`, `cargo run --features dev -- --write-mode=dev` | `qol dev` | direct, minus the hook step |
| `recompile-linked` | `bash scripts/recompile-linked-plugins.sh` | `qol dev` | folded in; script deleted |
| `lint` | `fmt-check` then `cargo clippy --all-targets --all-features -- -D warnings` | `qol check` | affected-crate scope instead of always-workspace |
| `lint-fix` | `cargo clippy --fix --allow-dirty --allow-staged` | `cargo clippy --fix` | bare cargo, no wrapper earns its keep |
| `fmt` | `cargo fmt --all` | `cargo fmt --all` | bare cargo |
| `fmt-check` | `cargo fmt --all --check` | `qol check` | `check/mod.rs:294` |
| `clean` | `cargo clean` | `qol clean` | direct (`commands/clean.rs:45`) |
| `clean-all` | `cargo clean` in qol-tray plus every `../*` sibling | `qol clean` | the `SIBLINGS` shell glob is a pre-monorepo relic; one workspace `target/` now |
| `ci-local` | fmt, clippy, test, then conditional Windows and macOS cross-checks | `qol check` plus a manual `clippy --target` | partial, see gap note |
| `install` | `cargo build --release --bins` then `target/release/qol-tray-install` | `qol install` | direct (`commands/install.rs:43-48`) |
| `install-dev` | same with `--features dev` and `--dev` | `qol install --dev` | **needs building**, gap 2 |
| `icons` | `bash scripts/generate-icons.sh` | undecided | **needs a decision**, gap 3 |
| `diagram` | `open`/`xdg-open` the HTML | open the HTML | already how `apps/qol-tray/README.md:20` documents it |
| `diagram-build` | `cd diagram && npm install && npm run build` | undecided | **needs a decision**, gap 4 |
| `release` | version bump, `chore(release): v$NEW` commit, `git tag v$NEW`, `git push --tags` | `plugin-version.yml` + `qol-tray-release.yml` | already superseded; running it would push a wrong tag |

### `plugins/*/Makefile`

| Target | Where | Before | After | Status |
|---|---|---|---|---|
| `build` | 7 plugins | `cargo build` | `qol build <name>` | direct |
| `dev` | all 10 | `cargo build`, `install -m 755 target/debug/$(BINARY)`, `mv` into the plugin root | `qol build <name>` | no staging needed: `execution_contract_tests.rs:78-107` asserts a dev-linked plugin resolves from the workspace `target/`, so `./$(BINARY)` was never the path the tray reads |
| `release` | all 10 | same with `--release` | release workflows | plugin artifacts are built by `release_candidate.py`, not locally |
| `test` | 9 plugins | `cargo test` | `qol check` | affected-crate scope |
| `check` | 7 plugins | `cargo check` | `qol check` | direct |
| `lint`, `lint-fix`, `fmt`, `fmt-check` | 7 plugins | `cargo fmt --all` / `cargo clippy --all-targets` | `qol check`, bare cargo for `--fix` | these already acted on the whole workspace despite living in a plugin |
| `clean` | 7 plugins | `cargo clean` | `qol clean` | direct |
| `ci-local` | 5 plugins | fmt, clippy, test, cross-checks | `qol check` plus manual `clippy --target` | partial, same as the tray row |
| `trace` | alt-tab | `qol trace alt-tab` | `qol trace alt-tab` | the target is already literally the command |
| `smoke-daemon` | bluetooth | `cargo build -p plugin-bluetooth` then `node tools/smoke-daemon.mjs` | `qol build bluetooth` then `node tools/smoke-daemon.mjs` | the script stays; only the wrapper goes |

Three rows do not survive as clean substitutions and become gaps below: `setup`, `install-dev`, and
the pair `icons` / `diagram-build`. `ci-local`'s cross-checks are noted rather than rebuilt.

## Gaps to close first

Four targets have no full `qol` equivalent. Each must be settled before the Makefile that hosts it
is deleted.

1. **`setup`.** `qol setup` is not a drop-in. It registers the Cargo.lock merge driver, sets
   `core.hooksPath`, and cargo-installs the CLI (`tools/qol-cli/src/setup.rs:27-57`). It never
   writes `.qol-tray-dev-hooks`. The Makefile is the **only** caller of
   `apps/qol-tray/scripts/dev-setup.sh`, and `qol dev` reads the hook file but never creates it
   (`commands/dev.rs:731-733`).

   **Disposition: delete the mechanism, do not replace it.** The pre-dev hook has never been used.
   `.qol-tray-dev-hooks` does not exist in this checkout, and `git check-ignore .qol-tray-dev-hooks`
   exits 1, so it is not ignored either, despite the generated file's own header line claiming
   "Gitignored - unique to your machine". Anyone who had run `make setup` would have found a
   machine-local executable sitting untracked at the repo root, one `git add -A` away from being
   committed. Remove `scripts/dev-setup.sh`, the `run_dev_hook` call and its helper
   (`commands/dev.rs:731-743`), and the `setup` / `$(DEV_HOOKS)` targets together.

   If the capability is ever genuinely wanted, it does not come back as a generated shell script.
   It belongs in `DevConfig` (`apps/qol-tray/src/dev/config/mod.rs:8`, persisted at
   `~/.config/qol-tray/dev/config.json`), which is already typed, already loaded by the tray, and
   already the home for machine-local dev settings. A declared list of commands there gets one
   labeled `qol dev` step per entry through the existing `run_dev_step` / `StepKind` pipeline, so a
   failing hook names itself. An opaque `set -euo pipefail` script gets exactly one step label and
   swallows the attribution, which is the weaker design even before the repo-root and
   not-actually-ignored problems.
2. **`install-dev`** (`cargo build --release --bins --features dev` then
   `qol-tray-install --dev`). Add a `--dev` flag to `qol install`; it has none today. The tray's own
   error message at `apps/qol-tray/src/installer/mod.rs:79` tells the user to run `make install-dev`,
   so that string changes with it.
3. **`icons`** (`bash scripts/generate-icons.sh`). The Makefile is the script's only caller. Either
   fold it into `qol build` as a subcommand or accept invoking the script by path.
4. **`diagram-build`** (`cd diagram && npm install && npm run build`). Rarely run. Simplest
   disposition is to move the two commands into a line of `apps/qol-tray/diagram/README.md` rather
   than build a CLI surface for them.

`ci-local`'s two cross-target checks (`--target x86_64-pc-windows-gnu`, `--target
x86_64-apple-darwin`) are not in `qol check` either, but they do not need a new surface: running
`clippy --target` before a push is already an established practice. Note it where `ci-local` is
documented rather than reimplementing it.

`scripts/recompile-linked-plugins.sh` becomes unreferenced once the tray Makefile goes
(`qol dev` covers it), so it is deleted in the same step rather than left orphaned.

## Steps

1. **Sweep the qol-skills marketplace first.** Standards evolution says the skills move before the
   repo does, and this is the highest-stakes step: a stale README misleads a human once, whereas a
   stale skill tells every future agent to run a command that no longer exists. Six live
   instructions across three plugins:

   | File | Line | Says |
   |---|---|---|
   | `qol-workflow/skills/readme/SKILL.md` | 103-104 | `make install` as the consumer path, `make dev` as the dev path |
   | `qol-workflow/skills/git-push/SKILL.md` | 41 | run `make build` / `make test` as the repo-native verification |
   | `qol-tray/agents/qol-tray-frontend.md` | 49 | "Run repo-native verification: `make build`, `make test`" |
   | `qol-tray/skills/qol-tray-feature-profile/SKILL.md` | 143-144 | `make build`, `make test` |
   | `qol-project/skills/qol-arch-cross-platform/SKILL.md` | 86 | "or run `make ci-local`" |
   | `qol-project/skills/qol-arch-cicd/SKILL.md` | 77-79 | an entire "`make ci-local`: developer-side parity" section |

   Replace each with `qol build` / `qol check`, and rewrite the `ci-local` section around
   `qol check` plus the manual `clippy --target` cross-checks. Bump and push `qol-workflow`,
   `qol-tray`, and `qol-project`. Leave `qol-plugin-pointz` alone (different repo, Flutter), and
   leave `qol-tray-core:170` and `qol-plugin-alt-tab:140` alone: both already say not to use a
   Makefile.
2. **Delete the dev-hooks mechanism** (gap 1): `apps/qol-tray/scripts/dev-setup.sh`, the
   `run_dev_hook` call and helper at `tools/qol-cli/src/commands/dev.rs:731-743`, and its call site.
   Nothing replaces it.
3. **Add `qol install --dev`**, and rewrite the `make install-dev` string in
   `apps/qol-tray/src/installer/mod.rs:79`.
4. **Settle `icons` and `diagram-build`** per the dispositions above.
5. **Delete the 10 plugin Makefiles.** Nothing needs replacing. Keep
   `plugins/bluetooth/tools/smoke-daemon.mjs`, which the deleted `smoke-daemon` target only wrapped,
   and note the two-command invocation wherever bluetooth's smoke test is documented.
6. **Delete `apps/qol-tray/Makefile`** and `apps/qol-tray/scripts/recompile-linked-plugins.sh`.
7. **Clean the live prose.**
   - `apps/qol-tray/docs/qol-commands.md:27` ("`make` remains authoritative ... not the release
     machinery") is now false. Replace with a pointer to the release workflows.
   - `apps/qol-tray/scripts/dev-setup.sh` mentions `make dev` and `make setup` in four user-facing
     `echo` lines.
   - `.claude/agent-memory/qol-tray-qol-tray-backend/MEMORY.md:14,22` record `make test` and
     `make build` behavior that will no longer exist.
8. **Leave the historical record alone.** ADRs and dated plans under `docs/` and
   `apps/qol-tray/docs/` describe what was true when written. `TRAY-6-devprod.md`,
   `TRAY-17-*.md`, `2026-06-20-removeapp.md`, `2026-07-08-plugin-controllers.md` and the rest keep
   their `make` references.

## Developer bootstrap on Linux, macOS, and Windows

Removing make does not change how a new developer starts, because make was never on that path.

`qol` does not ship natively anywhere, and it does not need to: it is built from the clone. The only
prerequisite is rustup, which is how Rust is installed on all three platforms anyway and which puts
`cargo` on PATH and `~/.cargo/bin` (where `qol` lands) on PATH with it. The first command is
therefore a `cargo` command, not a `qol` one:

```toml
# .cargo/config.toml, committed, so the alias exists the moment you clone
[alias]
setup = "install --path tools/qol-cli --locked --force --debug"
```

Cargo discovers `.cargo/config.toml` by walking up from the working directory, so `cargo setup`
resolves on a fresh clone with nothing installed but rustup. The bootstrap is one triple,
identical on Linux, macOS, and Windows:

```bash
cargo setup     # alias: builds and installs the qol CLI from tools/qol-cli
qol setup       # Cargo.lock merge driver, core.hooksPath, refresh the CLI
qol install     # build release binaries and install qol-tray
```

`cargo setup` and `qol setup` are not redundant: the alias only installs the binary, while
`qol setup` additionally registers the Cargo.lock merge driver and points `core.hooksPath` at
`.githooks` (`tools/qol-cli/src/setup.rs:33-36`). A make-based bootstrap would have needed make
installed first, which is the one tool of the three that genuinely does not ship on Windows.

The tray Makefile contributed nothing here. Its `setup` target only wrote the dead
`.qol-tray-dev-hooks` file, and `install` / `install-dev` are replaced by `qol install`
(plus `--dev`, gap 2). `apps/qol-tray/README.md:11-14` already documents exactly this triple.

Two cross-environment gaps exist, and both are orthogonal to make. They are **not** in this plan's
scope; they are recorded here because "how do devs on three OSes set up" is the question the
Makefile removal makes people ask.

1. **No `rust-toolchain.toml`.** Nothing pins rustc. Three developers on three platforms can hold
   three compiler versions while CI holds a fourth, and this repo gates on
   `clippy -D warnings`, where lint sets change between releases. A pinned toolchain file is the
   single highest-value addition to cross-platform setup, and it costs one file.
2. **No system-prerequisite check.** Despite the name, `doctor/checks/runtime_prereqs.rs` only
   verifies the plugins directory. Nothing checks the per-OS build dependencies (X11, xkb and D-Bus
   development packages on Linux, the Xcode command line tools on macOS, a linker toolchain on
   Windows). A developer on a fresh machine learns about them from a cargo build failure. Neither
   README states rustup as the prerequisite either, which is the same gap one level up.

The design principle for both: keep one identical command triple on every platform and push each OS
difference into `qol doctor` as data rather than into a README as prose. Doctor already carries the
`FixAction` mechanism (`runtime_prereqs.rs:48`), so a host-prerequisites check can name the missing
package and offer that platform's install command as a fix. That also satisfies the repo's own
README rule, which sends platform support to dynamic surfaces instead of prose. Note that
`.cargo/config.toml` already scopes its lld rustflags to `x86_64-unknown-linux-gnu`, so the
convention of keeping platform differences in one declarative place is established.

## Verification

Three lanes, and none of them needs the real GitHub.

**Host.** This is where the replacement claims are actually proven, because the guest never
compiles. `qol build`, `qol check`, `qol clean`, `qol install`, `qol install --dev`,
`qol trace alt-tab`, and `qol dev` all run green from a clean checkout, and
`find . -name Makefile -not -path '*/target/*'` returns nothing.

**Offline guest.** Proves the artifacts the new commands produce still boot as a real desktop
runtime:

```bash
qol env up linux/mint-cinnamon --dev-worktree <absolute-worktree>
qol env runs
qol env down <run-id>
```

The guest cannot reach GitHub even by accident. `tools/qol-cli/src/commands/env/mod.rs:1818` sets
`offline: dev_bundle.is_some()`, and `commands/emu/launch.rs:434` refuses a payload launch that is
not offline. `flows/envs/linux-mint-cinnamon.toml` sets `mounts.workspace = false`, so the guest
sees only the immutable read-only payload: no repo, no Makefiles, no cargo. What it can confirm is
that the tray service comes up, dev-linked plugins report ready, and a plugin action dispatches, all
from binaries built by `qol build` rather than by any Makefile target.

**CI.** Nothing here changes. No workflow under `.github/workflows/` invokes make, and neither do
the `.githooks`. The release path (`plugin-version.yml`, `qol-tray-release.yml`) is untouched
because `make release` is being deleted rather than run, so no real-GitHub rehearsal is required.

## Out of scope

`apps/qol-tray/src/features/task_runner/` and `apps/qol-tray/ui/views/task-runner/` use
`make build` as example user-configured task data. That is a plugin feature operating on the
user's own repo, not this repo's build system.
