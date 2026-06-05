# Alt Tab Plugin for QoL Tray

[![tests](https://github.com/qol-tools/plugin-alt-tab/actions/workflows/tests.yml/badge.svg)](https://github.com/qol-tools/plugin-alt-tab/actions/workflows/tests.yml)
[![lint](https://github.com/qol-tools/plugin-alt-tab/actions/workflows/lint.yml/badge.svg)](https://github.com/qol-tools/plugin-alt-tab/actions/workflows/lint.yml)

A window switcher with live previews for [QoL Tray](https://github.com/qol-tools/qol-tray).

## Quick start

Install from the [qol-tray](https://github.com/qol-tools/qol-tray) plugin store, or build from source:

```bash
git clone https://github.com/qol-tools/plugin-alt-tab
cd plugin-alt-tab
make build
```

## Diagnostics & Testing

To tail logs, verify multi-monitor placement, check opacity transitions, and detect any window system state divergences in real time:

```bash
make trace
```

This starts the trace aggregator tool, monitoring `/tmp/qol-altmon.log` and automatically validating window visibility and monitor placement alignments.

## License

PolyForm Noncommercial 1.0.0
