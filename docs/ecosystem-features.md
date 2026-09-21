# Ecosystem feature candidates

Brainstorm of features that enrich the qol ecosystem, grounded in the current
codebase (plugins, libs, tray, CLI) as of August 2026. Branch: `ecosystem-features`.

How to pick: each candidate lists what already exists to build on. Nothing here
is a commitment; it is a menu.

## P1 - New plugin candidates

| # | Plugin | What it does | Build on |
|---|--------|--------------|----------|
| 1 | `clipboard-history` | Clipboard ring with fuzzy search, pinning, paste via hotkey. Classic QoL; search already exists. | `qol-search`, `qol-hotkeys`, gpui surfaces |
| 2 | `text-expander` | Snippet expansion with trigger words; qol-keyremap's input-transform daemon is the natural host. | `qol-keyremap` daemon, `qol-hotkeys` |
| 3 | `power-battery` | Battery %, low-battery alerts, AC/battery power-profile switching (auto light/dark, DND). | `qol-os-themes`, `qol-runtime` push channel |
| 4 | `do-not-disturb` | Suppress notifications, block distracting apps on a schedule; pairs with power profiles. | `qol-runtime` push channel, `qol-os-themes` |
| 5 | `media-now-playing` | Detect now-playing (MPRIS / AppleScript), hotkey play/pause/next, album art and skip actions in qol-launcher. | `qol-launcher`, `qol-hotkeys`, `qol-terminal-sessions` discovery patterns |
| 6 | `window-tiling` | Tiling layouts beyond snapping (thirds, grids, workspaces), manual and hotkey-driven. | `qol-window-actions`, `qol-windowing` |
| 7 | `system-monitor` | CPU/RAM/disk/network into qol-launcher and tray status. | `qol-dev-env` inventory/run patterns, qol-launcher |
| 8 | `network-master` | Wi-Fi/VPN status, quick connect/disconnect, hotspot toggle in qol-launcher. | `qol-launcher`, daemon lifecycle pattern |
| 9 | `night-light` | Display color temperature on a schedule or manual toggle. | `qol-color` lib (already shared) |
| 10 | `keyboard-layout` | Per-app keyboard layout auto-switch and indicator. | `qol-keyremap` per-app policy infra |
| 11 | `session-restore` | Save/restore window layouts per profile; the flagship payoff of `qol-windowing`. | `qol-windowing`, profile export/import |
| 12 | `caffeine` | Keep-awake per session or per app (presentation mode). | `qol-os-themes`/power adapters, per-app policy |
| 13 | `focus-timer` | Pomodoro-style timer with notifications, tie-in to DND. | `qol-runtime` push channel |
| 14 | `qr-share` | Share text/URLs as QR from qol-launcher; `qr_code` field kind already exists. | `qol-config` `qr_code`, qol-launcher |
| 15 | `worktree-status` | Live board of worktrees/branches/dirty state with checkout actions; pairs with `qol-ide-checkout`. | `qol-ide-checkout` daemon, tray world canvas |
| 16 | `remote-clipboard` | Clipboard sync between PC and phone as a `qol-pointz` extension. | `qol-pointz` wire protocol, UDP discovery |

## P2 - Platform port matrix

Windows is a complete blank slate; macOS has gaps. Highest-reach ports first:

| Platform | Targets |
|----------|---------|
| Windows | `qol-window-actions`, `qol-alt-tab`, `qol-shot` (capture), `qol-keyremap` - the four biggest daily drivers |
| macOS | `qol-bluetooth`, `qol-controllers`, `qol-os-themes`, `qol-voice` (all currently "not implemented") |

## P3 - Ecosystem and dev experience

| # | Feature | What it does | Build on |
|---|---------|--------------|----------|
| 1 | `qol plugin new` scaffolder | Generate a new plugin from `qol-template` with correct layout and contract identity. | `qol-template` plugin, `plugin-layout.md`, `qol-workspace` |
| 2 | Plugin compatibility matrix | `qol doctor` cross-checks plugin `supported()` flags vs host OS, surfaces dead config. | `qol-cli doctor`, `qol-headless` convergence |
| 3 | E2E plugin test harness | Run per-plugin smoke tests inside `qol dev` guests (VM per plugin/OS). | `qol-dev-env`, `qol-dev-guest`, `qol-dev-orchestrator` |
| 4 | Feature catalog docs | READMEs are thin and have no roadmap sections; a catalog of plugins + gaps would let new sessions pick work fast. | this file, plugin READMEs |
| 5 | Plugin action catalog lib | Shared host-side actions (open URL, toast, set clipboard) so plugins stop re-implementing them. | `qol-plugin-api`, `qol-runtime` |

## Cross-cutting themes

- **Portability promise**: every feature should respect per-OS scope and profile
  sync (the "leaves no trace" goal). New plugins should declare capabilities and
  bail cleanly on unsupported platforms, like existing ones do.
- **Profile sync growth**: location-based auto-profile (home/work) and device
  overrides are natural next steps of the existing os/device scoped store.
- **Trace discipline**: new daemons should use `qol_runtime::probe!` from day one.
