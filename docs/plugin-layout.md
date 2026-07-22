# Plugin Source Layout

Plugin directories contain two different kinds of artifacts: code compiled into
the plugin binary and optional browser assets packaged for qol-tray. Their
locations are part of the architecture, not interchangeable styling choices.

## The two UI locations

| Path | Owner | Contents |
|---|---|---|
| `ui/` | qol-tray's web host | Packaged HTML, JavaScript, CSS, and schemas. The host discovers `ui/index.html` from the installed plugin root. |
| `src/ui/` | the plugin crate | Rust modules for native GPUI windows, views, toasts, panels, and presentation state. |

Root `ui/` is therefore correct for custom web pages such as keyremap and
lights. Native GPUI code never belongs there. Conversely, browser assets never
belong under `src/`.

## Canonical shape

Small plugins may contain only `src/main.rs` and `src/platform/`. Once a plugin
grows, its source root remains a composition layer:

```text
plugin-name/
├── plugin.toml
├── qol-config.toml          optional settings contract
├── qol-runtime.toml         optional runtime contract
├── ui/                      optional host-served web assets
└── src/
    ├── main.rs              thin binary entrypoint
    ├── lib.rs               crate composition and public facade
    ├── cli.rs               optional command adapter
    ├── app/                 long-running orchestration and daemon transport
    ├── config/              config model and config-specific actions
    ├── ui/                  native GPUI presentation
    ├── <capability>/         domain behavior owned by one capability
    │   └── platform/        OS strategy for that capability
    └── platform/            only OS capabilities shared across features
```

The dependency direction is:

```text
main.rs
├── cli.rs ───────┐
└── app/ ─────────┼──> capability modules ──> platform boundaries
                  └──> ui/ ─────────────────> qol-gpui
```

## Ownership rules

1. New Rust files directly under `src/` are limited to `main.rs`, `lib.rs`, and
   optional `cli.rs`. Implementation modules start inside their owning
   capability; `lib.rs` is the crate composition root and public facade.
2. A module with child modules uses `name/mod.rs`; do not keep the parent in
   `name.rs` while storing its children in `name/`.
3. OS conditionals stay behind the closest `platform/` strategy. Use the root
   `src/platform/` only when multiple capabilities consume the same OS service.
4. Native presentation shared by several actions belongs in `src/ui/`. A single
   self-contained feature may own its rendering inside its feature directory;
   do not add a second wrapper hierarchy merely to satisfy a name.
5. Keep stable library paths as facade re-exports when reorganizing published
   modules. Internal callers should use the new ownership path.
6. Avoid catch-all directories such as `helpers`, `common`, or `utils`. Name the
   capability or boundary that owns the code.
7. A relocation refactor changes paths only. Behavior changes and module moves
   are reviewed and verified separately.

The `qol-project:qol-arch-code` skill's edit hook enforces these rules
prospectively. Existing legacy root modules and hybrids remain editable until
their ownership refactor, while new layout debt is rejected.

## qol-shot reference shape

qol-shot demonstrates the grown-plugin form:

```text
src/
├── main.rs
├── lib.rs
├── cli.rs
├── app/
│   └── daemon/platform/
├── capture/
│   ├── gate/platform/
│   ├── screenshot.rs
│   ├── recording.rs
│   └── supporting capture modules
├── config/
├── ui/
│   ├── preview.rs
│   ├── pinned.rs
│   ├── region_selector/platform/
│   ├── capture_status.rs
│   ├── saved_toast.rs
│   └── settings_panel.rs
└── platform/
```
