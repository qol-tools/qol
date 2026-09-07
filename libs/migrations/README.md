<div align="center">

# QoL Migrations

On-disk and cloud-stored data migrations between [QoL Tray](../../apps/qol-tray) releases.

</div>

## Quick start

```toml
[dependencies]
qol-migrations.workspace = true
```

```rust
qol_migrations::run_pre_flight(&config_dir)?;

qol_migrations::run_post_auth(&MigrationContext { config_dir, github_token, http }).await?;
```

## About

Migration logic lives here and only here, so no feature module ever grows an "if old format do this" branch. The daemon calls one function per boot phase, and pruning a retired migration is a single `rm -rf` of one folder.

The two-phase boot model, the sliding release window, the pitfall guards behind each design choice, and the checklist for adding a migration are in [docs/design.md](./docs/design.md).

## License

PolyForm Noncommercial 1.0.0
