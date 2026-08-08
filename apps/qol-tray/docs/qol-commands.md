# qol commands

`qol` is the local dev orchestrator (built from `tools/qol-cli`). It is the
day-to-day entry point for building and running qol-tray and its sibling
plugins/crates from a checkout.

Install or refresh the helper:

```bash
cargo setup
```

Then run:

```bash
qol help        # the authoritative, always-current command + flag list
qol install     # build release binaries and install qol-tray
```

The command surface is defined in `tools/qol-cli` and printed by `qol help` -
that is the source of truth. This doc deliberately does not enumerate the
subcommands or flags, so it cannot drift as the CLI grows.

Two durable conventions worth knowing:

- Commands are quiet by default; `-v`/`--verbose` surfaces child-command output.
- Version bumps, tags, and releases are owned by `.github/workflows/plugin-version.yml` and `.github/workflows/qol-tray-release.yml`; `qol` never touches the release machinery.

Three operations stay direct invocations, not `qol` commands:

```bash
bash apps/qol-tray/scripts/generate-icons.sh
cd apps/qol-tray/diagram && npm install && npm run build
cd plugins/bluetooth && node tools/smoke-daemon.mjs
```

The bluetooth smoke test drives a locally built plugin; build it first with `qol build bluetooth`.
