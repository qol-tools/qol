# qol commands

Install or refresh the local helper:

```bash
cargo setup
```

`qol` is built from `tools/qol-cli`. Use `qol install` to install qol-tray.
Commands are quiet by default; pass `-v` or `--verbose` to show child command output.

## Commands

- `qol dev [worktree]` - run qol-tray in dev mode.
- `qol clean [name]` - clean qol-tray or a sibling plugin/crate.
- `qol install` - build release binaries and run the installer.
- `qol sync` - reserved; use `make sync` for now.
