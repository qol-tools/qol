# My Plugin

A current binary-first `qol-tray` plugin template for Rust plugins.

## Included Baseline

- Runtime binary entrypoint in [`src/main.rs`](src/main.rs)
- Platform-specific settings launchers in [`src/platform/`](src/platform/)
- Valid `plugin.toml` contract with `run` and `settings` actions
- Atomic `Makefile` install flow using `plugin-template.new`
- GitHub Actions `ci.yml`, `release.yml`, and `version.yml`
- `version.yml` wired to `qol-tools/qol-cicd` for shared version orchestration
- Contract validation test in `src/main.rs`

## Build Commands

- `make build`
- `make dev`
- `make release`
- `make test`
- `make check`
- `make lint`
- `make fmt-check`

## Customize

1. Rename `plugin-template` in `Cargo.toml`, `plugin.toml`, `.gitignore`, `Makefile`, and workflow artifact names.
2. Update `name`, `description`, supported `platforms`, and `[[dependencies.binaries]]` in `plugin.toml`.
3. Replace the `run` action body in `src/main.rs` with your plugin behavior.
4. Adjust `src/platform/` if your settings action or platform support differs.
5. Keep `Cargo.toml` and `plugin.toml` versions in sync.

## Contract Notes

- Commands must stay binary basenames only.
- If `runtime.actions` is present, every executable menu action must have a mapping.
- Add `[daemon]` only when the plugin actually needs a long-running process.
- Keep platform-specific behavior behind `src/platform/` or feature-owned platform modules.

## CI/CD

- `ci.yml` runs `cargo check` and `cargo test`.
- `version.yml` delegates semantic versioning and tag creation to `qol-cicd`.
- `release.yml` publishes tagged artifacts for Linux and macOS.

## License

PolyForm Noncommercial 1.0.0
