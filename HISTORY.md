# Monorepo Consolidation History

This monorepo was consolidated from 22 source repositories on 2026-06-01. Each
crate below traces its history to one of those repositories. At cutover the
originals are archived (made read-only); until then they remain the live
upstreams. Two repositories each contributed more than one crate: `qol-tray`
(the host `qol-tray` plus the `qol-cli` tool) and `qol-plugin-api` (the four
`qol-*` libraries in the plugin-api family).

## Applications

| Crate | Mono path | Source repo |
|-------|-----------|-------------|
| qol-tray | apps/qol-tray | https://github.com/qol-tools/qol-tray.git |

## Tools

| Crate | Mono path | Source repo |
|-------|-----------|-------------|
| qol-cli | tools/qol-cli | https://github.com/qol-tools/qol-tray.git |

## Libraries (standalone)

| Crate | Mono path | Source repo |
|-------|-----------|-------------|
| qol-color | libs/qol-color | https://github.com/qol-tools/qol-color.git |
| qol-config | libs/qol-config | https://github.com/qol-tools/qol-config.git |
| qol-frecency | libs/qol-frecency | https://github.com/qol-tools/qol-frecency.git |
| qol-migrations | libs/qol-migrations | https://github.com/qol-tools/qol-migrations.git |
| qol-platform | libs/qol-platform | https://github.com/qol-tools/qol-platform.git |
| qol-runtime | libs/qol-runtime | https://github.com/qol-tools/qol-runtime.git |
| qol-search | libs/qol-search | https://github.com/qol-tools/qol-search.git |
| qol-wasm | libs/qol-wasm | https://github.com/qol-tools/qol-wasm.git |

## Libraries (qol-plugin-api family)

These four crates were workspace members of a single source repository and
share one origin.

| Crate | Mono path | Source repo |
|-------|-----------|-------------|
| qol-app-icon | libs/qol-app-icon | https://github.com/qol-tools/qol-plugin-api.git |
| qol-gpui | libs/qol-gpui | https://github.com/qol-tools/qol-plugin-api.git |
| qol-plugin-api | libs/qol-plugin-api | https://github.com/qol-tools/qol-plugin-api.git |
| qol-plugin-daemon | libs/qol-plugin-daemon | https://github.com/qol-tools/qol-plugin-api.git |

## Plugins

| Crate | Mono path | Source repo |
|-------|-----------|-------------|
| plugin-alt-tab | plugins/plugin-alt-tab | https://github.com/qol-tools/plugin-alt-tab.git |
| plugin-claude-sessions | _removed 2026-06-02 (archived upstream)_ | https://github.com/qol-tools/plugin-claude-sessions.git |
| plugin-ide-checkout | plugins/plugin-ide-checkout | https://github.com/qol-tools/plugin-ide-checkout.git |
| plugin-keyremap | plugins/plugin-keyremap | https://github.com/qol-tools/plugin-keyremap.git |
| plugin-kitty | _removed 2026-06-02 (archived upstream)_ | https://github.com/qol-tools/plugin-kitty.git |
| plugin-launcher | plugins/plugin-launcher | https://github.com/qol-tools/plugin-launcher.git |
| plugin-lights | plugins/plugin-lights | https://github.com/qol-tools/plugin-lights.git |
| plugin-os-themes | plugins/plugin-os-themes | https://github.com/qol-tools/plugin-os-themes.git |
| plugin-pointz | plugins/plugin-pointz | https://github.com/qol-tools/plugin-pointz.git |
| plugin-screen-recorder | plugins/plugin-screen-recorder | https://github.com/qol-tools/plugin-screen-recorder.git |
| plugin-template | plugins/plugin-template | https://github.com/qol-tools/plugin-template.git |
| plugin-window-actions | plugins/plugin-window-actions | https://github.com/qol-tools/plugin-window-actions.git |

`plugin-kitty` and `plugin-claude-sessions` were part of the 22-repo
consolidation on 2026-06-01, then archived upstream and removed from the tree on
2026-06-02 (commit `e898146`). Their rows are kept for provenance.
