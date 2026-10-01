<div align="center">

# qol

[![tests](https://github.com/qol-tools/qol/actions/workflows/ci.yml/badge.svg)](https://github.com/qol-tools/qol/actions/workflows/ci.yml)
[![Release Plugin](https://github.com/qol-tools/qol/actions/workflows/release.yml/badge.svg)](https://github.com/qol-tools/qol/actions/workflows/release.yml)
[![QoL Tray Release](https://github.com/qol-tools/qol/actions/workflows/qol-tray-release.yml/badge.svg)](https://github.com/qol-tools/qol/actions/workflows/qol-tray-release.yml)

A portable quality-of-life layer for any computer you sit down at.

<img src="https://raw.githubusercontent.com/KMRH47/KMRH47/main/shots/hero.webp" width="100%" alt="The qol launcher on Linux Mint, searching for qol, term and fir">

</div>

## Quick start

### Install

[Prebuilt tray downloads](https://github.com/qol-tools/qol/releases/latest)

### Develop

```bash
cargo setup
qol dev
```

> [!TIP]
> `cargo setup` installs the `qol` dev CLI; `qol dev` builds the plugins and runs the tray. Run `qol help` for the rest.

## About

Install once, configure once. Your plugins, keybindings, and settings follow you: boot the tray on any machine and it becomes yours; pull away and the host is left as you found it.

## Plugins

<img src="https://raw.githubusercontent.com/KMRH47/KMRH47/main/shots/settings.webp" width="100%" alt="The qol settings panel, stepping through the Alt Tab, Display, OS Themes, Shot and Window Actions pages">

| Plugin | What it does | Runs on |
|---|---|---|
| Alt Tab | Better alt-tab experience with window previews | Linux, macOS |
| Bluetooth | Reliably reconnect the Bluetooth devices you choose | Linux, macOS |
| CLI Sessions | Always-on-top overview of live CLI sessions (Claude Code, Codex, any command) | Linux, macOS |
| Controllers | Inspect connected game controllers and apply relevant driver-specific fixes | Linux |
| Display | Display brightness, gamma, and mode control | Linux, macOS |
| IDE Checkout | Local HTTP API for a browser extension to check out a git branch and open it in a configured app | Linux, macOS |
| Key Remap | Remap keyboard and mouse shortcuts (Ctrl->Cmd, etc.) | macOS |
| Launcher | Universal search with action modifiers | Linux, macOS |
| Lights | Control lights through backend adapters | Linux, macOS |
| OS Themes | Cursor effects and OS-wide theming | Linux |
| PointZerver | Control your PC from mobile devices | Linux, macOS |
| QoL Memory | Long-context memory: retrieve settled facts from your agent session history | Linux, macOS |
| Remove App | Uninstall an app and its leftovers | Linux, macOS |
| Shot | Capture screenshots and record screen regions | Linux, macOS |
| Sound | Choose where sound plays and how loud | Linux |
| Voice | Transcribe speech and route completed turns to live terminal sessions | Linux |
| Window Actions | Window snapping, centering, and multi-monitor management | Linux, macOS |

## License

PolyForm Noncommercial 1.0.0
