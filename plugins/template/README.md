# Plugin Template

[![tests](https://github.com/qol-tools/plugin-template/actions/workflows/tests.yml/badge.svg)](https://github.com/qol-tools/plugin-template/actions/workflows/tests.yml)
[![lint](https://github.com/qol-tools/plugin-template/actions/workflows/lint.yml/badge.svg)](https://github.com/qol-tools/plugin-template/actions/workflows/lint.yml)

A starting point for building [QoL Tray](https://github.com/qol-tools/qol-tray) plugins.

## Quick start

Click "Use this template" on GitHub, or:

```bash
gh repo create my-plugin --template qol-tools/plugin-template
cd my-plugin
make build
```

## About

Ships a binary entrypoint, platform-specific settings launchers, a valid `plugin.toml` contract, and GitHub Actions CI / release / version workflows wired to [qol-cicd](https://github.com/qol-tools/qol-cicd).

## License

PolyForm Noncommercial 1.0.0
