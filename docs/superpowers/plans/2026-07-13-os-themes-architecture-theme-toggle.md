# os-themes Linux Architecture Seams + Light/Dark Toggle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give plugin-os-themes explicit Linux seams (display server, desktop environment) and ship a light/dark theme toggle with a Cinnamon backend.

**Architecture:** OS level keeps the compile-time `platform/` strategy pattern. Inside Linux, two runtime-selected layers are added: `cursor/platform/linux/display/` (X11 vs Wayland, detected via `XDG_SESSION_TYPE`) and `theme/platform/linux/backends/` (Cinnamon/GNOME/KDE, detected via `XDG_CURRENT_DESKTOP`). The 1080-line `x11.rs` is split move-only into concern modules.

**Tech Stack:** Rust, x11/xcursor/xfixes FFI (existing), `gsettings` via spawned process (no new crate dependencies).

## Global Constraints

- Workspace root: `/media/kmrh47/WD_SN850X/Git/qol-monorepo`. Run all cargo commands from there with `-p plugin-os-themes`.
- No code comments anywhere. Existing comments in moved code must be preserved as-is (move-only), but new code gets none.
- Never use the em-dash character in any file or commit message.
- Conventional one-liner commits, subject <= 72 chars, no AI attribution, no co-authors, no pushing.
- Gate before every commit: `cargo fmt -p plugin-os-themes --check && cargo clippy -p plugin-os-themes --all-targets -- -D warnings && cargo test -p plugin-os-themes`. All must pass with real output.
- No `#[cfg(target_os)]` outside `platform/mod.rs` wiring. No `_ =>` arms on enum matches (string matches may use catch-alls).
- Table-driven tests with context in assertions; snake_case descriptive test names.
- The spec is `docs/superpowers/specs/2026-07-13-os-themes-architecture-theme-toggle-design.md`. One amendment made during planning: the dark-name heuristic must validate candidates against installed themes, because the live machine's theme is `Mint-Y-Dark-Pink` (infix `-Dark-`, so a plain suffix swap is wrong). Commit sequencing is also revised to 4 commits so no commit ships dead code.
- Live machine facts (verified 2026-07-13): `XDG_CURRENT_DESKTOP=X-Cinnamon`, `XDG_SESSION_TYPE=x11`, `gsettings get org.cinnamon.desktop.interface gtk-theme` = `'Mint-Y-Dark-Pink'`, `gsettings get org.cinnamon.theme name` = `'Mint-Y-Dark-Pink'`.

---

### Task 1: Display-server seam for cursor code

**Files:**
- Create: `plugins/os-themes/src/cursor/platform/linux/display/mod.rs`
- Move: `plugins/os-themes/src/cursor/platform/linux/x11.rs` -> `plugins/os-themes/src/cursor/platform/linux/display/x11.rs` (content untouched)
- Modify: `plugins/os-themes/src/cursor/platform/linux/mod.rs:1-4` (module list)
- Modify: `plugins/os-themes/src/cursor/platform/linux/runtime.rs:88-93` (`open_session`)

**Interfaces:**
- Consumes: existing `super::x11::CursorSession::open(scale_factor)`.
- Produces: `display::ensure_cursor_support() -> anyhow::Result<()>`, `display::x11::CursorSession` (same type, new path). `DisplayServer` enum with `X11 | Wayland | Unknown`.

- [ ] **Step 1: Write the failing test**

Create `display/mod.rs` with only the test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_type_detection_table() {
        let cases = [
            (Some("x11"), DisplayServer::X11),
            (Some("X11"), DisplayServer::X11),
            (Some("wayland"), DisplayServer::Wayland),
            (Some(" Wayland "), DisplayServer::Wayland),
            (Some("tty"), DisplayServer::Unknown),
            (Some(""), DisplayServer::Unknown),
            (None, DisplayServer::Unknown),
        ];
        for (input, expected) in cases {
            assert_eq!(from_session_type(input), expected, "input: {input:?}");
        }
    }
}
```

Add `mod display;` to `linux/mod.rs` (keep `mod x11;` for now so the tree still compiles).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p plugin-os-themes session_type_detection_table`
Expected: compile FAIL, `from_session_type` and `DisplayServer` not found.

- [ ] **Step 3: Implement display/mod.rs**

Full file (above the test module):

```rust
use anyhow::{bail, Result};

pub(super) mod x11;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DisplayServer {
    X11,
    Wayland,
    Unknown,
}

fn from_session_type(value: Option<&str>) -> DisplayServer {
    let Some(value) = value.map(str::trim) else {
        return DisplayServer::Unknown;
    };
    if value.eq_ignore_ascii_case("wayland") {
        DisplayServer::Wayland
    } else if value.eq_ignore_ascii_case("x11") {
        DisplayServer::X11
    } else {
        DisplayServer::Unknown
    }
}

pub(super) fn ensure_cursor_support() -> Result<()> {
    let session_type = std::env::var("XDG_SESSION_TYPE").ok();
    match from_session_type(session_type.as_deref()) {
        DisplayServer::Wayland => {
            bail!("cursor effects require X11; Wayland is not supported yet")
        }
        DisplayServer::X11 | DisplayServer::Unknown => Ok(()),
    }
}
```

`pub(super) mod x11;` will not resolve until Step 4's move; do Steps 3 and 4 together before compiling.

- [ ] **Step 4: Move x11.rs and rewire**

```bash
git mv plugins/os-themes/src/cursor/platform/linux/x11.rs plugins/os-themes/src/cursor/platform/linux/display/x11.rs
```

In `linux/mod.rs` delete the `mod x11;` line (keep `mod display;` from Step 1).

In `runtime.rs` change `open_session` to:

```rust
fn open_session(config: &Config) -> Result<Session> {
    super::display::ensure_cursor_support()?;
    eprintln!("[shake-to-grow] started mode=tree");
    Ok(Session::Tree(super::display::x11::CursorSession::open(
        config.scale_factor,
    )?))
}
```

`x11.rs` items referenced from `runtime.rs` are currently `pub(super)` or `pub(crate)`; after the move `pub(super)` now means `display`, so bump any `pub(super)` items in `x11.rs` that `runtime.rs` uses (at minimum `CursorSession`, its methods, and `ScaleUpdate`-adjacent types if any live there) to `pub(crate)`. Compile errors will list them exactly; fix only what the compiler names.

- [ ] **Step 5: Run the gate**

Run: `cargo fmt -p plugin-os-themes --check && cargo clippy -p plugin-os-themes --all-targets -- -D warnings && cargo test -p plugin-os-themes`
Expected: PASS, including `session_type_detection_table` and the existing 8 motion tests.

- [ ] **Step 6: Commit**

```bash
git add -A plugins/os-themes && git commit -m "refactor(os-themes): seam cursor code behind a display-server layer"
```

---

### Task 2: Split display/x11.rs into concern modules (move-only)

**Files:**
- Move: `display/x11.rs` -> `display/x11/mod.rs` plus create `display/x11/{session.rs,sampling.rs,source.rs,animation.rs}` (all paths under `plugins/os-themes/src/cursor/platform/linux/`)
- Modify: Xephyr rig `#[path]` include at `/tmp/claude-1000/-media-kmrh47-WD-SN850X-Git-qol-monorepo/5d184a27-0bdb-41af-8aab-1c1151b95e8c/scratchpad/os-themes-verify/src/main.rs`

**Interfaces:**
- Consumes: the items listed in the mapping below (current `x11.rs` top-level items).
- Produces: identical public surface via `display/x11/mod.rs` re-exports; `CursorSession` path unchanged for `runtime.rs`.

- [ ] **Step 1: Create the module skeleton**

`display/x11/mod.rs` keeps: the `use` header, all consts (`MAX_CURSOR_DIMENSION`, `XFIXES_CURSOR_NOTIFY`, `XFIXES_DISPLAY_CURSOR_NOTIFY`, `XFIXES_DISPLAY_CURSOR_NOTIFY_MASK`, `FRAME_TABLE_CAP`, `MAX_SOURCE_FRAMES`), the `unsafe extern "C"` FFI block, and the shared structs `BaseCursor`, `CursorRaster`, `CursorImage`, `ScaledCursor`. Add:

```rust
mod animation;
mod sampling;
mod session;
mod source;

pub(crate) use session::CursorSession;
```

- [ ] **Step 2: Move items by this exact mapping (bodies unchanged)**

| Module | Items |
|---|---|
| `session.rs` | `CursorSession` struct + `impl CursorSession` + `impl Drop`, `log_x_error`, `apply_to_tree`, `clear_children`, `restore_root_cursor`, `window_children`, `subscribe_cursor_notifications`, `sync`, `live_refresh_enabled` |
| `sampling.rs` | `load_live_cursor_image`, `same_cursor_image`, `is_our_enlarged_cursor`, `applied_cursor_is_scaled_variant`, `is_empty_cursor`, `log_cursor_image`, `cursor_hash`, `raster_hash`, `pixel_signature`, `hash_cursor_value`, `checked_pixel_count`, `copy_cursor_name`, `cursor_raster_from_xcursor_image` |
| `source.rs` | `CATALOG_SHAPE_NAMES`, `ShapeCatalog` + impl, `build_frame_table`, `cursor_rasters_from_images`, `load_base_cursor`, `preferred_source_size`, `with_best_source`, `named_cursor_frames`, `fallback_base_source`, `matches_base_cursor`, `source_improves_cursor`, `load_named_cursor_raster`, `load_named_cursor_frames`, `thin_frames` |
| `animation.rs` | `make_cursor_from_frames`, `scale_cursor_for_display`, `make_cursor_at_scale`, `best_cursor_size`, `scaled_dimension`, `scaled_raster_hotspot`, `sanitize_hotspot`, `sanitize_dimension` |

Each submodule starts with `use super::*;` so shared structs, consts, FFI, and sibling items resolve without touching bodies. Mark moved items `pub(super)` where cross-submodule calls require it (the compiler names each one). Struct fields accessed across submodules become `pub(super)`.

- [ ] **Step 3: Run the gate**

Run: `cargo fmt -p plugin-os-themes --check && cargo clippy -p plugin-os-themes --all-targets -- -D warnings && cargo test -p plugin-os-themes`
Expected: PASS. `cargo fmt` will reformat the new files; run `cargo fmt -p plugin-os-themes` first, then the gate.

- [ ] **Step 4: Rig regression in Xephyr**

Update the rig's include to `#[path = ".../src/cursor/platform/linux/display/x11/mod.rs"] mod x11;` (submodules resolve relative to `mod.rs`, so no other rig change). Rebuild and run per the `os-themes-xephyr-verification-rig` memory: `detector-feel`, `lowres-repro`, `regrow-repro`, `anim-repro` against Xephyr `:9` with a persistent xeyes client.
Expected: identical results to pre-split (all detector sims correct; static shapes `source=96x96x1`; spinner `source=96x96x51`, `animated=true`; custom bitmap `source=none`; no pixelated regrow).

- [ ] **Step 5: Commit**

```bash
git add -A plugins/os-themes && git commit -m "refactor(os-themes): split x11 cursor module by concern"
```

---

### Task 3: Theme toggle (API, naming, Cinnamon backend, action wiring)

**Files:**
- Modify: `plugins/os-themes/src/theme/mod.rs` (currently `pub mod platform;` only)
- Modify: `plugins/os-themes/src/theme/platform/mod.rs` (new trait, drop allows)
- Delete: `plugins/os-themes/src/theme/platform/linux.rs`
- Create: `plugins/os-themes/src/theme/platform/linux/mod.rs`
- Create: `plugins/os-themes/src/theme/platform/linux/gsettings.rs`
- Create: `plugins/os-themes/src/theme/platform/linux/backends/{mod.rs,naming.rs,cinnamon.rs,kde.rs}`
- Modify: `plugins/os-themes/src/theme/platform/{macos.rs,windows.rs}` (new trait stubs)
- Modify: `plugins/os-themes/src/config.rs` (two fields + contract test)
- Modify: `plugins/os-themes/qol-config.toml` (theme section)
- Modify: `plugins/os-themes/plugin.toml` (toggle-theme action)
- Modify: `plugins/os-themes/src/app/mod.rs` (CLI dispatch)
- Modify: `plugins/os-themes/src/daemon.rs` + `src/app/daemon_run.rs` (socket dispatch)

**Interfaces:**
- Consumes: `crate::config::Config`, `qol_config` contract loading.
- Produces: `theme::ColorScheme { Light, Dark }` with `fn opposite(self) -> Self`; `theme::toggle(config: &Config) -> Result<ColorScheme>`; trait `ThemePlatform { fn current_scheme(&self) -> Result<ColorScheme>; fn apply_scheme(&self, target: ColorScheme, config: &Config) -> Result<()>; }`; `daemon::Command::ToggleTheme`.

- [ ] **Step 1: Write failing naming tests**

Create `backends/naming.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ColorScheme;

    fn installed(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn classify_detects_dark_segment() {
        let cases = [
            ("Mint-Y", ColorScheme::Light),
            ("Mint-Y-Dark", ColorScheme::Dark),
            ("Mint-Y-Dark-Pink", ColorScheme::Dark),
            ("Adwaita-dark", ColorScheme::Dark),
            ("Darkroom", ColorScheme::Light),
            ("foo_dark", ColorScheme::Dark),
        ];
        for (theme, expected) in cases {
            assert_eq!(classify(theme), expected, "theme: {theme}");
        }
    }

    #[test]
    fn resolve_prefers_configured_names() {
        let themes = installed(&["foo", "bar"]);
        let resolved = resolve("anything", ColorScheme::Dark, "", "bar", &themes).unwrap();
        assert_eq!(resolved, "bar");
        let resolved = resolve("anything", ColorScheme::Light, "foo", "", &themes).unwrap();
        assert_eq!(resolved, "foo");
    }

    #[test]
    fn resolve_derives_counterpart_from_installed_names() {
        let themes = installed(&[
            "Mint-Y-Pink",
            "Mint-Y-Dark-Pink",
            "Adwaita",
            "Adwaita-dark",
        ]);
        let cases = [
            ("Mint-Y-Dark-Pink", ColorScheme::Light, "Mint-Y-Pink"),
            ("Mint-Y-Pink", ColorScheme::Dark, "Mint-Y-Dark-Pink"),
            ("Adwaita", ColorScheme::Dark, "Adwaita-dark"),
            ("Adwaita-dark", ColorScheme::Light, "Adwaita"),
        ];
        for (current, target, expected) in cases {
            let resolved = resolve(current, target, "", "", &themes).unwrap();
            assert_eq!(resolved, expected, "current: {current}, target: {target:?}");
        }
    }

    #[test]
    fn resolve_keeps_current_when_already_matching() {
        let resolved = resolve("Mint-Y-Dark", ColorScheme::Dark, "", "", &installed(&[])).unwrap();
        assert_eq!(resolved, "Mint-Y-Dark");
    }

    #[test]
    fn resolve_errors_when_no_counterpart_installed() {
        let result = resolve("Custom-Theme", ColorScheme::Dark, "", "", &installed(&["Custom-Theme"]));
        assert!(result.is_err(), "expected no dark counterpart for Custom-Theme");
    }
}
```

Wire the module tree minimally so the test compiles later: `theme/platform/linux/mod.rs` declares `mod backends;` and `mod gsettings;`, `backends/mod.rs` declares `mod naming;` (plus the rest as they land).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p plugin-os-themes naming`
Expected: compile FAIL (`classify`, `resolve`, `theme::ColorScheme` missing).

- [ ] **Step 3: Implement theme/mod.rs and the trait**

`theme/mod.rs`:

```rust
pub mod platform;

use anyhow::Result;

use crate::config::Config;
use platform::{Platform, ThemePlatform};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorScheme {
    Light,
    Dark,
}

impl ColorScheme {
    fn opposite(self) -> Self {
        match self {
            ColorScheme::Light => ColorScheme::Dark,
            ColorScheme::Dark => ColorScheme::Light,
        }
    }
}

pub fn toggle(config: &Config) -> Result<ColorScheme> {
    let platform = Platform;
    let target = platform.current_scheme()?.opposite();
    platform.apply_scheme(target, config)?;
    Ok(target)
}
```

`theme/platform/mod.rs` (full replacement; the doc comment block and both `#[allow(...)]` attributes are removed because the module now has real consumers):

```rust
use anyhow::Result;

use crate::config::Config;
use crate::theme::ColorScheme;

pub trait ThemePlatform {
    fn current_scheme(&self) -> Result<ColorScheme>;
    fn apply_scheme(&self, target: ColorScheme, config: &Config) -> Result<()>;
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::Platform;
#[cfg(target_os = "macos")]
pub use macos::Platform;
#[cfg(target_os = "windows")]
pub use windows::Platform;
```

`macos.rs` (and `windows.rs` identically, with "Windows" in the messages):

```rust
use anyhow::{anyhow, Result};

use crate::config::Config;
use crate::theme::ColorScheme;

use super::ThemePlatform;

pub struct Platform;

impl ThemePlatform for Platform {
    fn current_scheme(&self) -> Result<ColorScheme> {
        Err(anyhow!("theme switching is not implemented on macOS"))
    }

    fn apply_scheme(&self, _target: ColorScheme, _config: &Config) -> Result<()> {
        Err(anyhow!("theme switching is not implemented on macOS"))
    }
}
```

- [ ] **Step 4: Implement naming.rs (above the tests)**

```rust
use anyhow::{bail, Result};

use crate::theme::ColorScheme;

pub(super) fn classify(theme: &str) -> ColorScheme {
    if theme
        .split(['-', '_'])
        .any(|part| part.eq_ignore_ascii_case("dark"))
    {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    }
}

pub(super) fn resolve(
    current: &str,
    target: ColorScheme,
    configured_light: &str,
    configured_dark: &str,
    installed: &[String],
) -> Result<String> {
    let configured = match target {
        ColorScheme::Light => configured_light,
        ColorScheme::Dark => configured_dark,
    };
    if !configured.is_empty() {
        return Ok(configured.to_string());
    }
    if classify(current) == target {
        return Ok(current.to_string());
    }
    let counterpart = match target {
        ColorScheme::Light => light_variant(current, installed),
        ColorScheme::Dark => dark_variant(current, installed),
    };
    match counterpart {
        Some(name) => Ok(name),
        None => bail!(
            "no installed {target:?} counterpart found for theme {current:?}; set the theme names in settings"
        ),
    }
}

fn light_variant(theme: &str, installed: &[String]) -> Option<String> {
    let stripped: Vec<&str> = theme
        .split('-')
        .filter(|part| !part.eq_ignore_ascii_case("dark"))
        .collect();
    let candidate = stripped.join("-");
    installed.iter().find(|name| **name == candidate).cloned()
}

fn dark_variant(theme: &str, installed: &[String]) -> Option<String> {
    let parts: Vec<&str> = theme.split('-').collect();
    let mut candidates = Vec::new();
    for position in 1..=parts.len() {
        let mut with_dark: Vec<&str> = parts.clone();
        with_dark.insert(position, "Dark");
        candidates.push(with_dark.join("-"));
    }
    candidates.push(format!("{theme}-dark"));
    candidates
        .into_iter()
        .find(|candidate| installed.iter().any(|name| name == candidate))
}
```

- [ ] **Step 5: Run naming tests**

Run: `cargo test -p plugin-os-themes naming`
Expected: PASS (5 tests). The rest of the module tree must exist by now (Steps 6-7 files can be stubbed in the same working state; compile requires them since `linux/mod.rs` declares them).

- [ ] **Step 6: Implement gsettings.rs, backends, detection**

`theme/platform/linux/gsettings.rs`:

```rust
use std::process::Command;

use anyhow::{bail, Context, Result};

pub(super) fn get(schema: &str, key: &str) -> Result<String> {
    let output = Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .context("failed to run gsettings")?;
    if !output.status.success() {
        bail!(
            "gsettings get {schema} {key} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(unquote(String::from_utf8_lossy(&output.stdout).trim()))
}

pub(super) fn set(schema: &str, key: &str, value: &str) -> Result<()> {
    let status = Command::new("gsettings")
        .args(["set", schema, key, value])
        .status()
        .context("failed to run gsettings")?;
    if !status.success() {
        bail!("gsettings set {schema} {key} {value} failed");
    }
    Ok(())
}

fn unquote(value: &str) -> String {
    value
        .strip_prefix('\'')
        .and_then(|inner| inner.strip_suffix('\''))
        .unwrap_or(value)
        .to_string()
}
```

`theme/platform/linux/backends/mod.rs`:

```rust
mod cinnamon;
mod kde;
mod naming;

use anyhow::Result;

use crate::config::Config;
use crate::theme::ColorScheme;

pub(super) use cinnamon::Cinnamon;
pub(super) use kde::Kde;

pub(super) trait DesktopBackend {
    fn current_scheme(&self) -> Result<ColorScheme>;
    fn apply(&self, target: ColorScheme, config: &Config) -> Result<()>;
}

pub(super) fn installed_themes() -> Vec<String> {
    let mut roots = vec![
        std::path::PathBuf::from("/usr/share/themes"),
        std::path::PathBuf::from("/usr/local/share/themes"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(std::path::PathBuf::from(home).join(".themes"));
    }
    let mut names = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                if let Ok(name) = entry.file_name().into_string() {
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
            }
        }
    }
    names
}
```

`theme/platform/linux/backends/cinnamon.rs` (the shell theme is best-effort by design: a missing counterpart for the desktop shell theme must not fail the whole toggle, the GTK theme is the contract):

```rust
use anyhow::Result;

use crate::config::Config;
use crate::theme::ColorScheme;

use super::super::gsettings;
use super::{installed_themes, naming, DesktopBackend};

const INTERFACE_SCHEMA: &str = "org.cinnamon.desktop.interface";
const SHELL_SCHEMA: &str = "org.cinnamon.theme";

pub(in super::super) struct Cinnamon;

impl DesktopBackend for Cinnamon {
    fn current_scheme(&self) -> Result<ColorScheme> {
        Ok(naming::classify(&gsettings::get(INTERFACE_SCHEMA, "gtk-theme")?))
    }

    fn apply(&self, target: ColorScheme, config: &Config) -> Result<()> {
        let installed = installed_themes();
        let current = gsettings::get(INTERFACE_SCHEMA, "gtk-theme")?;
        let gtk_theme = naming::resolve(
            &current,
            target,
            &config.gtk_theme_light,
            &config.gtk_theme_dark,
            &installed,
        )?;
        gsettings::set(INTERFACE_SCHEMA, "gtk-theme", &gtk_theme)?;
        apply_shell_theme(target, config, &installed);
        Ok(())
    }
}

fn apply_shell_theme(target: ColorScheme, config: &Config, installed: &[String]) {
    let Ok(current) = gsettings::get(SHELL_SCHEMA, "name") else {
        return;
    };
    let resolved = naming::resolve(
        &current,
        target,
        &config.gtk_theme_light,
        &config.gtk_theme_dark,
        installed,
    );
    match resolved {
        Ok(name) => {
            if let Err(error) = gsettings::set(SHELL_SCHEMA, "name", &name) {
                eprintln!("[os-themes] shell theme not updated: {error:#}");
            }
        }
        Err(error) => eprintln!("[os-themes] shell theme not updated: {error:#}"),
    }
}
```

`theme/platform/linux/backends/kde.rs`:

```rust
use anyhow::{anyhow, Result};

use crate::config::Config;
use crate::theme::ColorScheme;

use super::DesktopBackend;

pub(in super::super) struct Kde;

impl DesktopBackend for Kde {
    fn current_scheme(&self) -> Result<ColorScheme> {
        Err(anyhow!("KDE theme switching is not implemented yet"))
    }

    fn apply(&self, _target: ColorScheme, _config: &Config) -> Result<()> {
        Err(anyhow!("KDE theme switching is not implemented yet"))
    }
}
```

`theme/platform/linux/mod.rs` (replaces the deleted `linux.rs`):

```rust
mod backends;
mod gsettings;

use anyhow::{bail, Result};

use crate::config::Config;
use crate::theme::ColorScheme;

use super::ThemePlatform;
use backends::DesktopBackend;

pub struct Platform;

impl ThemePlatform for Platform {
    fn current_scheme(&self) -> Result<ColorScheme> {
        detect_backend()?.current_scheme()
    }

    fn apply_scheme(&self, target: ColorScheme, config: &Config) -> Result<()> {
        detect_backend()?.apply(target, config)
    }
}

fn detect_backend() -> Result<Box<dyn DesktopBackend>> {
    let raw = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    backend_for(&raw)
}

fn backend_for(raw: &str) -> Result<Box<dyn DesktopBackend>> {
    for part in raw.split(':') {
        match part.trim().to_ascii_lowercase().as_str() {
            "x-cinnamon" | "cinnamon" => return Ok(Box::new(backends::Cinnamon)),
            "kde" => return Ok(Box::new(backends::Kde)),
            _ => {}
        }
    }
    bail!("unsupported desktop environment for theme switching: {raw:?}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_detection_table() {
        let cases = [
            ("X-Cinnamon", true),
            ("cinnamon", true),
            ("KDE", true),
            ("ubuntu:GNOME", false),
            ("", false),
            ("Hyprland", false),
        ];
        for (input, expected) in cases {
            assert_eq!(backend_for(input).is_ok(), expected, "input: {input}");
        }
    }
}
```

(`ubuntu:GNOME` flips to `true` in Task 5 when the GNOME backend lands; the test case is updated there.)

- [ ] **Step 7: Config fields and contracts**

`src/config.rs`: add to the struct, after `restore_steps`:

```rust
    pub gtk_theme_light: String,
    pub gtk_theme_dark: String,
```

Extend `log_config` with ` gtk_light={:?} gtk_dark={:?}` and the two fields. In the contract test add:

```rust
        assert_eq!(defaults.gtk_theme_light, "");
        assert_eq!(defaults.gtk_theme_dark, "");
```

`qol-config.toml`: append:

```toml
[section.theme]
label = "Light/Dark Theme"
description = "Toggle the desktop between light and dark. Leave the theme names empty to derive the counterpart from the current theme's name."
actions = ["toggle-theme"]

[field.gtk_theme_light]
type = "string"
label = "Light Theme"
section = "theme"
default = ""

[field.gtk_theme_dark]
type = "string"
label = "Dark Theme"
section = "theme"
default = ""
```

`plugin.toml`: after `[action.run]` block add:

```toml
[action.toggle-theme]
label = "Toggle Light/Dark"
args = ["toggle-theme"]
```

- [ ] **Step 8: Wire CLI and daemon dispatch**

`src/app/mod.rs` match arm, after `Some("settings")`:

```rust
        Some("toggle-theme") => toggle_theme(),
```

and below `run`:

```rust
fn toggle_theme() -> Result<()> {
    let config = crate::config::load();
    let scheme = crate::theme::toggle(&config)?;
    eprintln!("[os-themes] applied {scheme:?} theme");
    Ok(())
}
```

`src/daemon.rs`: add `ToggleTheme` to `Command` and in `parse_command` add before the fallback:

```rust
        "toggle-theme" => ReadResult::Command(Command::ToggleTheme),
```

`src/app/daemon_run.rs` `handle_daemon_commands` match gains (the match is exhaustive, the compiler forces this):

```rust
            daemon::Command::ToggleTheme => {
                let config = crate::config::load();
                match crate::theme::toggle(&config) {
                    Ok(scheme) => eprintln!("[os-themes] applied {scheme:?} theme"),
                    Err(error) => eprintln!("[os-themes] toggle-theme failed: {error:#}"),
                }
            }
```

This is required because `daemon.command == runtime.command` makes the tray route every action through the socket while the daemon is alive (see memory: single-binary daemon must handle all actions).

- [ ] **Step 9: Run the gate**

Run: `cargo fmt -p plugin-os-themes --check && cargo clippy -p plugin-os-themes --all-targets -- -D warnings && cargo test -p plugin-os-themes`
Expected: PASS. New tests: 5 naming + 1 detection + updated contract test.

- [ ] **Step 10: Commit**

```bash
git add -A plugins/os-themes && git commit -m "feat(os-themes): toggle light/dark theme on cinnamon"
```

---

### Task 4: Live verification on Cinnamon (no commit unless fixes needed)

**Files:** none (verification only).

- [ ] **Step 1: Direct CLI toggle**

```bash
cargo build -p plugin-os-themes
gsettings get org.cinnamon.desktop.interface gtk-theme
QOL_TRAY_PLUGIN_ID=plugin-os-themes ./target/debug/plugin-os-themes toggle-theme
gsettings get org.cinnamon.desktop.interface gtk-theme
gsettings get org.cinnamon.theme name
```

Expected: `Mint-Y-Dark-Pink` -> `Mint-Y-Pink` on both schemas, desktop visibly flips to light. Note: the binary may exit via the daemon ping path only for `run`; `toggle-theme` dispatches directly in `app::run`, no daemon needed. If `config::load` requires more env than `QOL_TRAY_PLUGIN_ID`, copy the env pattern from the Xephyr rig memory (sandboxed `HOME` is NOT wanted here; we want the real config).

- [ ] **Step 2: Toggle back**

Run the toggle again. Expected: back to `Mint-Y-Dark-Pink` on both schemas, desktop flips dark. The `-Dark` insertion candidate list must find `Mint-Y-Dark-Pink` from `Mint-Y-Pink`.

- [ ] **Step 3: Daemon-routed toggle**

Stage the dev binary (`qol build plugin-os-themes`), restart the daemon via the tray Recompile button or `qol dev`, then trigger the `Toggle Light/Dark` action from the qol-tray menu. Expected: theme flips; no CLI fallback errors in tray logs.

- [ ] **Step 4: Report**

Report observed transitions to the user before proceeding to Task 5. If any step failed, fix within Task 3's scope and amend that commit (verify HEAD is ours first per the verify-head-before-amend memory).

---

### Task 5: GNOME backend (best-effort, not locally verifiable)

**Files:**
- Create: `plugins/os-themes/src/theme/platform/linux/backends/gnome.rs`
- Modify: `backends/mod.rs` (declare + re-export), `linux/mod.rs` (detection arm + test case)

**Interfaces:**
- Consumes: `gsettings`, `naming::resolve`, `naming::classify`, `installed_themes`, `DesktopBackend`.
- Produces: `backends::Gnome`.

- [ ] **Step 1: Update the detection test**

In `linux/mod.rs` `backend_detection_table`, change `("ubuntu:GNOME", false)` to `("ubuntu:GNOME", true)` and add `("GNOME", true)`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p plugin-os-themes backend_detection_table`
Expected: FAIL on `ubuntu:GNOME`.

- [ ] **Step 3: Implement gnome.rs**

```rust
use anyhow::Result;

use crate::config::Config;
use crate::theme::ColorScheme;

use super::super::gsettings;
use super::{installed_themes, naming, DesktopBackend};

const INTERFACE_SCHEMA: &str = "org.gnome.desktop.interface";

pub(in super::super) struct Gnome;

impl DesktopBackend for Gnome {
    fn current_scheme(&self) -> Result<ColorScheme> {
        let scheme = gsettings::get(INTERFACE_SCHEMA, "color-scheme")?;
        if scheme == "prefer-dark" {
            Ok(ColorScheme::Dark)
        } else {
            Ok(ColorScheme::Light)
        }
    }

    fn apply(&self, target: ColorScheme, config: &Config) -> Result<()> {
        let color_scheme = match target {
            ColorScheme::Light => "default",
            ColorScheme::Dark => "prefer-dark",
        };
        gsettings::set(INTERFACE_SCHEMA, "color-scheme", color_scheme)?;
        apply_gtk_theme(target, config);
        Ok(())
    }
}

fn apply_gtk_theme(target: ColorScheme, config: &Config) {
    let Ok(current) = gsettings::get(INTERFACE_SCHEMA, "gtk-theme") else {
        return;
    };
    let resolved = naming::resolve(
        &current,
        target,
        &config.gtk_theme_light,
        &config.gtk_theme_dark,
        &installed_themes(),
    );
    match resolved {
        Ok(name) => {
            if let Err(error) = gsettings::set(INTERFACE_SCHEMA, "gtk-theme", &name) {
                eprintln!("[os-themes] gtk theme not updated: {error:#}");
            }
        }
        Err(error) => eprintln!("[os-themes] gtk theme not updated: {error:#}"),
    }
}
```

On GNOME the authoritative switch is `color-scheme` (libadwaita apps follow it); the legacy `gtk-theme` swap is best-effort, mirroring the Cinnamon shell-theme policy.

In `backends/mod.rs` add `mod gnome;` and `pub(super) use gnome::Gnome;`. In `linux/mod.rs` `backend_for` add the arm `"gnome" => return Ok(Box::new(backends::Gnome)),`.

- [ ] **Step 4: Run the gate**

Run: `cargo fmt -p plugin-os-themes --check && cargo clippy -p plugin-os-themes --all-targets -- -D warnings && cargo test -p plugin-os-themes`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A plugins/os-themes && git commit -m "feat(os-themes): add gnome theme toggle backend"
```
