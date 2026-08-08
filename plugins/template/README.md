<div align="center">

# Plugin Template

A starting point for building [QoL Tray](../../apps/qol-tray) plugins.

</div>

## Quick start

```bash
cp -r plugins/template plugins/my-plugin
```

Replace every occurrence of the template identity in the copy, then `qol build my-plugin`.

## About

Ships a binary entrypoint, a per-OS settings launcher for each supported platform, and a valid `plugin.toml` contract, so the copy builds and loads before you have written any of its own behavior.

## License

PolyForm Noncommercial 1.0.0
