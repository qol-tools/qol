---
paths:
  - "src/**/*.rs"
  - "Cargo.toml"
---

# Path convention: qol-config is the single source for config and data dirs

`qol-config` is the one source of truth for the `qol-tray`-namespaced **config and
data** directories. New components MUST resolve these dirs through `qol-config`,
never by re-deriving `dirs::config_dir()` / `dirs::data_local_dir()` and joining a
`qol-tray` literal inline.

## The API and the mapping

| Bucket | qol-config function | Resolver it wraps | Namespace |
| --- | --- | --- | --- |
| Data | `qol_config::data_dir()` | `dirs::data_local_dir().or_else(dirs::data_dir)` | `qol-tray/` |
| Data subdir | `qol_config::data_subdir(name)` | `data_dir()/name` | `qol-tray/<name>` |
| Config | `qol_config::config_dir()` | `dirs::config_dir()` | `qol-tray/` |

- The namespace literal lives once, as `qol_config::NAMESPACE` (`"qol-tray"`).
- `data_dir()` is canonical; `base_data_dir()` is a `#[doc(hidden)]` alias kept for
  back-compat. Both return `Option<PathBuf>`.
- `config_roots()` and `plugin_config_paths_from_env()` are the existing shared
  surface (qol-gpui and the config plugins depend on their byte-identical install
  search order) and are NOT changed by this convention.

## There is no state dir

The qol-universe does not use an XDG state dir. Run state (e.g. `run.log`,
`report.json`) stays under `target/`, not under a `data`/`config` dir. Do not add a
state-dir resolver.

## Residual `qol-tray` namespace literals NOT covered

This rule is scoped to config/data dirs. The following `qol-tray` literals are
acknowledged residuals and are intentionally NOT routed through `qol-config`:

- **qol-tray test-only override branch** (`apps/qol-tray/src/paths.rs`). The
  `QOL_TRAY_TEST_PATH_ROOT` thread-local override stack and its guards stay in
  qol-tray; only the production join delegates to `qol_config::data_dir()` /
  `qol_config::config_dir()`. The override branch joins `qol_config::NAMESPACE`
  directly rather than calling the resolver, so external integration tests that set
  `QOL_TRAY_TEST_PATH_ROOT` keep working.
- **Log dirs**, whose platform conventions differ from `data_dir` and so cannot
  route through it:
  - macOS `apps/qol-tray/src/logging/platform/macos.rs` - `Library/Logs/qol-tray`
    (macOS `data_dir` is `Application Support`, not `Library/Logs`; redirecting
    would relocate logs).
  - Windows `apps/qol-tray/src/logging/platform/windows.rs` - `qol-tray/logs`.
  - temp-dir fallback `apps/qol-tray/src/logging/file_logger.rs` -
    `temp_dir()/qol-tray/logs`.

  These are log-class literals; converging them is deferred to a future
  `qol_config` log-namespace helper (out of scope).
