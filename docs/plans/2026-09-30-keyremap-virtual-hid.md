# Key Remap Virtual HID Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Key Remap keeps remapping keys while any macOS app holds Secure Input, by moving keyboard input onto the pqrs virtual keyboard behind a root helper, with today's event tap as the fallback strategy and a toast that names the app holding Secure Input.

**Architecture:** A root LaunchDaemon (`qol-keyremap hid-helper`) seizes the physical keyboards and forwards raw HID key events to the user daemon over a unix socket. The daemon runs the existing `process_key_event` engine and sends back the HID usages to press; the helper posts them through the pqrs daemon to the DriverKit virtual keyboard. When the helper is absent, stale, or silent for 200 ms, the keyboards are released and the event tap strategy takes over unchanged.

**Tech Stack:** Rust 2021, raw FFI to IOKit, Carbon (TIS, UCKeyTranslate) and CoreGraphics, `core-foundation 0.10`, `objc2-app-kit`, `serde_json` lines over `std::os::unix::net::UnixStream`, launchd.

**Spec:** `docs/specs/2026-09-30-keyremap-virtual-hid-design.md`

## Global Constraints

- The dependency is Karabiner-DriverKit-VirtualHIDDevice, package 8.6.0, driver 1.8.0, client protocol 7. Keyremap never installs or talks to Karabiner-Elements.
- pqrs socket: `/Library/Application Support/org.pqrs/tmp/rootonly/karabiner_virtual_hid_device_service.sock`.
- pqrs daemon binary: `/Library/Application Support/org.pqrs/Karabiner-DriverKit-VirtualHIDDevice/Applications/Karabiner-VirtualHIDDevice-Daemon.app/Contents/MacOS/Karabiner-VirtualHIDDevice-Daemon`.
- pqrs manager binary: `/Applications/.Karabiner-VirtualHIDDevice-Manager.app/Contents/MacOS/Karabiner-VirtualHIDDevice-Manager`, run with `activate`.
- pqrs launchd label: `org.pqrs.service.daemon.Karabiner-VirtualHIDDevice-Daemon`. Keyremap's own label for it: `com.qol-tools.keyremap.vhid-daemon`, used only when the pqrs label is not loaded.
- Helper binary: `/Library/PrivilegedHelperTools/com.qol-tools.keyremap.hid-helper`, `root:wheel`, mode 0755. Helper LaunchDaemon: `/Library/LaunchDaemons/com.qol-tools.keyremap.hid-helper.plist`, label `com.qol-tools.keyremap.hid-helper`.
- Helper socket: `/var/run/com.qol-tools.keyremap.hid-helper.sock`, owned by the console user, mode 0600, peer uid checked with `getpeereid`.
- Helper protocol version: `1`. Daemon heartbeat every 50 ms. Helper releases every seized keyboard after 200 ms of silence.
- Everything new in keyremap is macOS-only and lives under `plugins/keyremap/src/platform/macos/`. No `cfg(target_os)` outside a `platform/` `mod.rs`.
- `doctor` is read-only. A repair is its own command, named in `.with_fix(...)`.
- Output only through `log::` in library code; prints only in `cli.rs`.
- No new crates. Raw FFI for IOKit, Carbon and SystemConfiguration.
- Never knowingly duplicate code: the tray's `UCKeyTranslate` FFI moves into `qol_hotkeys::layout` and both callers use it.
- No comments except where the code cannot say it.
- Never use the em-dash character anywhere.
- Commit direct to `main`, one commit per task, no AI attribution in commit messages.
- After any `Cargo.toml` change (Tasks 1 and 3), run `cargo hakari generate && cargo hakari manage-deps` and `cargo metadata --locked --format-version 1 > /dev/null`, and commit `libs/workspace-hack/` and `Cargo.lock` with the task. CI fails a manifest change that skipped either.
- Modules land before their callers. Declare a new module as `#[allow(dead_code)] mod name;` when clippy flags items a later task uses. Task 14 removes every such attribute and the final gate passes without them.
- Linux compile check: `cargo check -p qol-keyremap --target x86_64-unknown-linux-gnu`. If it fails inside `ring` (a known cross-compile limit on this Mac), skip it and rely on the Ubuntu CI job. The new probe types in `platform/mod.rs` follow `ConfigInspection`, which the Linux adapter also never constructs; if Ubuntu clippy flags them, give them the same treatment `ConfigInspection` gets.
- Toast text, verbatim from the spec:
  - `event_tap`: title "Key Remap and qol hotkeys are paused: <App> has turned on Secure Input."
  - `virtual_hid`: title "qol hotkeys are paused: <App> has turned on Secure Input."

## Review Focus

1. **Keyboard hot-plugged while seized.** A person plugs in a USB keyboard while the helper holds the built-in one. The new keyboard must be seized on arrival and go through the rules, and unplugging a seized keyboard must not leave its keys held. Pinned by `watchdog::tests::device_arrival_while_seized_asks_for_a_seize` and `report::tests::unplugging_releases_what_the_device_held` (Task 7), and the Task 15 hot-plug check.
2. **Stuck keys when the strategy switches.** The daemon dies while Cmd (from a Ctrl+C rule) is held on the virtual keyboard. After the helper releases, nothing may stay pressed. Pinned by `report::tests::clear_releases_every_page_that_had_keys` (Task 7) and `keyboard::tests::reset_forgets_held_keys` (Task 11).
3. **Overlapping sessions.** `qol dev` reload starts a second daemon before the first exits. The newest session wins, the old one's heartbeats are ignored, and the old one's disconnect must not release the new one's seize. Pinned by `watchdog::tests::a_stale_session_cannot_keep_or_drop_the_seize` (Task 7).
4. **Input Monitoring not granted to the helper.** The helper then sees no keys. It must never seize in that state, or the keyboard dies. Pinned by `watchdog::tests::no_input_monitoring_never_seizes` (Task 7) and the doctor text in Task 14.
5. **TIS called off the main thread.** macOS aborts the process when TIS is called off the main thread. Layout reads happen only on the daemon's main thread and in doctor. Pinned by `layout::tests::store_serves_readers_without_touching_tis` (Task 4) and the Task 12 wiring, which calls `LayoutStore::refresh` only from `app::run`.

## File Structure

| Path | Responsibility |
| --- | --- |
| `libs/plugin-api/src/manifest/schema.rs` | `SystemDependency` type, `Dependencies.system` |
| `libs/plugin-api/src/manifest/validation/dependency_rules.rs` | System dependency validation, `parse_version` |
| `libs/plugin-api/src/manifest/validation/manifest_rules.rs` | `PluginManifest::parse_and_validate(&str)` |
| `libs/hotkeys/src/keycode/backends/carbon.rs` | `PhysicalLayout`, HID usage to keycode table |
| `libs/hotkeys/src/layout/` | Shared `UCKeyTranslate` FFI (`KeyLayout`) |
| `apps/tray/src/hotkeys/capture/platform/macos/layout.rs` | Rebuilt on `qol_hotkeys::layout` |
| `plugins/keyremap/src/platform/macos/layout/mod.rs` | Character to key strokes table, `LayoutStore` |
| `plugins/keyremap/src/platform/macos/virtual_hid/client/{mod,frame,request}.rs` | pqrs client protocol 7 |
| `plugins/keyremap/src/platform/macos/virtual_hid/{mod,driver}.rs` | Paths, labels, driver and daemon probes |
| `plugins/keyremap/src/platform/macos/hid_helper/{mod,protocol,watchdog,report,devices,console,server,install}.rs` | Root helper |
| `plugins/keyremap/src/platform/macos/input/mod.rs` | `Strategy`, `StrategyCell`, `MarkerBook` |
| `plugins/keyremap/src/platform/macos/input/backends/event_tap.rs` | Today's key handling moved out of `tap.rs` |
| `plugins/keyremap/src/platform/macos/input/backends/virtual_hid/{mod,keyboard,fn_keys}.rs` | Daemon side of the virtual HID strategy |
| `plugins/keyremap/src/platform/macos/secure_input/{mod,warning}.rs` | Secure Input probe and toast state machine |
| `plugins/keyremap/src/platform/mod.rs` | New adapter methods and doctor probe types |
| `plugins/keyremap/src/cli.rs` | New commands and five doctor checks |

Verification gate for every task, run from the repo root:

```bash
cargo fmt --all
cargo clippy -p <crate> --all-targets -- -D warnings
cargo test -p <crate>
```

---

### Task 1: System dependencies in the plugin manifest

**Files:**
- Modify: `libs/plugin-api/src/manifest/schema.rs:211-222`
- Modify: `libs/plugin-api/src/manifest/mod.rs:10-13`
- Modify: `libs/plugin-api/src/manifest/validation/dependency_rules.rs`
- Modify: `libs/plugin-api/src/manifest/validation/manifest_rules.rs:50-60`
- Modify: `libs/plugin-api/src/manifest/validation_tests.rs:607-640`
- Modify: `plugins/keyremap/plugin.toml`, `plugins/keyremap/Cargo.toml`, `plugins/keyremap/src/main.rs`
- Modify: `docs/plugin-contract.md` (after the `[[dependencies.binaries]]` bullet)

**Interfaces:**
- Produces: `qol_plugin_api::manifest::SystemDependency { name, platforms, min_version, license, url }` (all `String` / `Vec<String>`), `Dependencies.system: Vec<SystemDependency>`, `qol_plugin_api::manifest::parse_version(&str) -> Option<(u64, u64, u64)>`, `PluginManifest::parse_and_validate(raw: &str) -> anyhow::Result<PluginManifest>`.

- [ ] **Step 1: Write the failing tests**

Append to `mod dependency_rules` in `libs/plugin-api/src/manifest/validation_tests.rs`, and add `system: Vec::new(),` to the two existing `Dependencies { binaries: ... }` literals in that module:

```rust
    fn vhid() -> SystemDependency {
        SystemDependency {
            name: "Karabiner-DriverKit-VirtualHIDDevice".to_string(),
            platforms: vec!["macos".to_string()],
            min_version: "8.6.0".to_string(),
            license: "Unlicense".to_string(),
            url: "https://github.com/pqrs-org/Karabiner-DriverKit-VirtualHIDDevice".to_string(),
        }
    }

    fn with_system(system: SystemDependency) -> PluginManifest {
        PluginManifest {
            dependencies: Some(Dependencies {
                binaries: Vec::new(),
                system: vec![system],
            }),
            ..base_manifest()
        }
    }

    #[test]
    fn validate_accepts_a_complete_system_dependency() {
        assert!(with_system(vhid()).validate().is_ok());
    }

    #[test]
    fn validate_rejects_a_system_dependency_with_an_unparseable_min_version() {
        let error = with_system(SystemDependency {
            min_version: "8.6".to_string(),
            ..vhid()
        })
        .validate()
        .unwrap_err();
        assert!(error.to_string().contains("min_version"), "{error}");
    }

    #[test]
    fn validate_rejects_a_system_dependency_with_an_unknown_platform() {
        let error = with_system(SystemDependency {
            platforms: vec!["darwin".to_string()],
            ..vhid()
        })
        .validate()
        .unwrap_err();
        assert!(error.to_string().contains("darwin"), "{error}");
    }

    #[test]
    fn validate_rejects_a_system_dependency_with_blank_fields() {
        for blank in [
            SystemDependency { name: " ".to_string(), ..vhid() },
            SystemDependency { license: String::new(), ..vhid() },
            SystemDependency { url: String::new(), ..vhid() },
            SystemDependency { platforms: Vec::new(), ..vhid() },
        ] {
            assert!(with_system(blank).validate().is_err());
        }
    }

    #[test]
    fn parse_version_reads_three_numeric_parts_only() {
        assert_eq!(parse_version("8.6.0"), Some((8, 6, 0)));
        assert_eq!(parse_version("10.20.30"), Some((10, 20, 30)));
        assert_eq!(parse_version("8.6"), None);
        assert_eq!(parse_version("8.6.0.1"), None);
        assert_eq!(parse_version("8.x.0"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn parse_and_validate_reads_system_dependencies_from_toml() {
        let raw = r#"
[plugin]
id = "qol-example"
uid = "e1bc6f9b-95e0-46c5-951b-6cc5de5c6d87"
name = "Example"
description = "Example"
version = "1.0.0"

[menu]
label = "Example"
items = []

[[dependencies.system]]
name = "Karabiner-DriverKit-VirtualHIDDevice"
platforms = ["macos"]
min_version = "8.6.0"
license = "Unlicense"
url = "https://github.com/pqrs-org/Karabiner-DriverKit-VirtualHIDDevice"
"#;
        let manifest = PluginManifest::parse_and_validate(raw).unwrap();
        let system = &manifest.dependencies.unwrap().system;
        assert_eq!(system, &vec![vhid()]);
    }
```

Add `SystemDependency` and `parse_version` to the `use` list at the top of `validation_tests.rs` (it imports from `crate::manifest`).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p qol-plugin-api dependency_rules`
Expected: compile error, `cannot find type SystemDependency`.

- [ ] **Step 3: Implement**

`schema.rs`, replace the `Dependencies` struct and add the new type after `BinaryDependency`:

```rust
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Dependencies {
    #[serde(default)]
    pub binaries: Vec<BinaryDependency>,
    #[serde(default)]
    pub system: Vec<SystemDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SystemDependency {
    pub name: String,
    pub platforms: Vec<String>,
    pub min_version: String,
    pub license: String,
    pub url: String,
}
```

`dependency_rules.rs`, full file:

```rust
use crate::manifest::{BinaryDependency, Dependencies, SystemDependency};
use anyhow::{bail, Result};

const MANIFEST_PLATFORMS: [&str; 3] = ["linux", "macos", "windows"];

impl Dependencies {
    pub fn validate(&self) -> Result<()> {
        for binary in &self.binaries {
            binary.validate()?;
        }
        for system in &self.system {
            system.validate()?;
        }
        Ok(())
    }
}

impl BinaryDependency {
    pub fn validate(&self) -> Result<()> {
        super::command_rules::validate_command_name("dependencies.binaries.name", &self.name)
    }
}

impl SystemDependency {
    pub fn validate(&self) -> Result<()> {
        for (field, value) in [
            ("name", &self.name),
            ("license", &self.license),
            ("url", &self.url),
        ] {
            if value.trim().is_empty() {
                bail!("dependencies.system.{field} must not be empty");
            }
        }
        if self.platforms.is_empty() {
            bail!("dependencies.system.platforms must name at least one platform");
        }
        if let Some(unknown) = self
            .platforms
            .iter()
            .find(|platform| !MANIFEST_PLATFORMS.contains(&platform.as_str()))
        {
            bail!("dependencies.system.platforms has unknown platform {unknown:?}");
        }
        if parse_version(&self.min_version).is_none() {
            bail!(
                "dependencies.system.min_version {:?} is not MAJOR.MINOR.PATCH",
                self.min_version
            );
        }
        Ok(())
    }
}

pub fn parse_version(raw: &str) -> Option<(u64, u64, u64)> {
    let mut parts = raw.split('.').map(|part| part.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

pub(super) fn validate_optional_dependencies(dependencies: Option<&Dependencies>) -> Result<()> {
    let Some(dependencies) = dependencies else {
        return Ok(());
    };

    dependencies.validate()
}
```

Re-export `parse_version`: in `validation/mod.rs` add `pub use dependency_rules::parse_version;` (make `mod dependency_rules` visible to that line if it is private; it stays private, only the function is re-exported). In `manifest/mod.rs` add `SystemDependency` to the `pub use schema::{...}` list and add `pub use validation::parse_version;`.

`manifest_rules.rs`, replace `load_and_validate` with:

```rust
    pub fn parse_and_validate(raw: &str) -> Result<Self> {
        let manifest: Self = toml::from_str(raw).context("parse plugin manifest")?;
        manifest.validate().context("validate plugin manifest")?;
        Ok(manifest)
    }

    pub fn load_and_validate(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        Self::parse_and_validate(&raw).with_context(|| format!("load {}", path.display()))
    }
```

- [ ] **Step 4: Declare the dependency in keyremap**

Append to `plugins/keyremap/plugin.toml`:

```toml
[[dependencies.system]]
name = "Karabiner-DriverKit-VirtualHIDDevice"
platforms = ["macos"]
min_version = "8.6.0"
license = "Unlicense"
url = "https://github.com/pqrs-org/Karabiner-DriverKit-VirtualHIDDevice"
```

In `plugins/keyremap/Cargo.toml` move `qol-plugin-api.workspace = true` from `[dev-dependencies]` to `[dependencies]` (doctor reads the embedded manifest at runtime in Task 14).

Add to the `tests` module in `plugins/keyremap/src/main.rs`:

```rust
    #[test]
    fn manifest_declares_the_virtual_hid_driver_as_a_macos_system_dependency() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
        let system = manifest
            .dependencies
            .expect("plugin.toml must declare dependencies")
            .system;
        let driver = system
            .iter()
            .find(|dependency| dependency.name == "Karabiner-DriverKit-VirtualHIDDevice")
            .expect("the virtual HID driver must be declared");
        assert_eq!(driver.platforms, vec!["macos".to_string()]);
        assert_eq!(driver.min_version, "8.6.0");
        assert_eq!(driver.license, "Unlicense");
    }
```

Add to `docs/plugin-contract.md`, directly after the `[[dependencies.binaries]]` bullet:

```markdown
- `[[dependencies.system]]` (`SystemDependency`): `name`, `platforms` (subset of
  `linux`, `macos`, `windows`), `min_version` (`MAJOR.MINOR.PATCH`), `license`,
  `url`. Declares software the user installs outside qol, such as a driver. The
  host does not install it; the plugin's `doctor` reports whether it is present.
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p qol-plugin-api && cargo test -p qol-keyremap manifest`
Expected: PASS, including the six new plugin-api tests and the new keyremap manifest test.

- [ ] **Step 6: Commit**

```bash
git add libs/plugin-api plugins/keyremap/plugin.toml plugins/keyremap/Cargo.toml plugins/keyremap/src/main.rs docs/plugin-contract.md Cargo.lock libs/workspace-hack
git commit -m "feat(plugin-api): declare system dependencies in plugin manifests"
```

---

### Task 2: HID usage to macOS keycode table

**Files:**
- Modify: `libs/hotkeys/src/keycode/backends/carbon.rs` (append after `key_name`, tests into its existing `mod tests`)

**Interfaces:**
- Produces: `qol_hotkeys::macos_keycode::PhysicalLayout { Ansi, Iso, Jis }`, `PhysicalLayout::from_layout_type(u32) -> PhysicalLayout`, `from_hid_usage(usage: u16, layout: PhysicalLayout) -> Option<u16>`, `to_hid_usage(code: u16, layout: PhysicalLayout) -> Option<u16>`.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `carbon.rs`:

```rust
    const LAYOUTS: [PhysicalLayout; 3] =
        [PhysicalLayout::Ansi, PhysicalLayout::Iso, PhysicalLayout::Jis];

    #[test]
    fn every_named_keycode_survives_a_trip_through_hid() {
        for layout in LAYOUTS {
            for code in 0..0x80u16 {
                if key_name(code) == "unknown" {
                    continue;
                }
                let usage = to_hid_usage(code, layout)
                    .unwrap_or_else(|| panic!("{} has no HID usage", key_name(code)));
                assert_eq!(from_hid_usage(usage, layout), Some(code), "{layout:?} {code:#04x}");
            }
        }
    }

    #[test]
    fn iso_keyboards_swap_the_grave_and_section_keys() {
        assert_eq!(from_hid_usage(0x35, PhysicalLayout::Ansi), Some(ANSI_GRAVE));
        assert_eq!(from_hid_usage(0x64, PhysicalLayout::Ansi), Some(ISO_SECTION));
        assert_eq!(from_hid_usage(0x35, PhysicalLayout::Iso), Some(ISO_SECTION));
        assert_eq!(from_hid_usage(0x64, PhysicalLayout::Iso), Some(ANSI_GRAVE));
        assert_eq!(to_hid_usage(ISO_SECTION, PhysicalLayout::Iso), Some(0x35));
        assert_eq!(to_hid_usage(ANSI_GRAVE, PhysicalLayout::Iso), Some(0x64));
    }

    #[test]
    fn duplicate_usages_map_back_to_their_canonical_usage() {
        assert_eq!(from_hid_usage(0x46, PhysicalLayout::Ansi), Some(0x69));
        assert_eq!(to_hid_usage(0x69, PhysicalLayout::Ansi), Some(0x68));
        assert_eq!(from_hid_usage(0x32, PhysicalLayout::Ansi), Some(ANSI_BACKSLASH));
        assert_eq!(to_hid_usage(ANSI_BACKSLASH, PhysicalLayout::Ansi), Some(0x31));
    }

    #[test]
    fn modifiers_and_caps_lock_have_usages() {
        assert_eq!(from_hid_usage(0xE0, PhysicalLayout::Ansi), Some(0x3B));
        assert_eq!(from_hid_usage(0xE6, PhysicalLayout::Ansi), Some(0x3D));
        assert_eq!(from_hid_usage(0x39, PhysicalLayout::Ansi), Some(0x39));
        assert_eq!(from_hid_usage(0x00, PhysicalLayout::Ansi), None);
    }

    #[test]
    fn layout_type_fourccs_pick_the_physical_layout() {
        assert_eq!(PhysicalLayout::from_layout_type(0x4953_4F20), PhysicalLayout::Iso);
        assert_eq!(PhysicalLayout::from_layout_type(0x4A49_5320), PhysicalLayout::Jis);
        assert_eq!(PhysicalLayout::from_layout_type(0x414E_5349), PhysicalLayout::Ansi);
        assert_eq!(PhysicalLayout::from_layout_type(0), PhysicalLayout::Ansi);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p qol-hotkeys carbon`
Expected: compile error, `cannot find type PhysicalLayout`.

- [ ] **Step 3: Implement**

Append to `carbon.rs` above `#[cfg(test)]`. The order matters: `to_hid_usage` returns the first usage listed for a keycode, so canonical usages come before their duplicates (F13 to F15 at 0x68 to 0x6A before Print Screen, Scroll Lock and Pause at 0x46 to 0x48; backslash 0x31 before non-US hash 0x32).

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalLayout {
    Ansi,
    Iso,
    Jis,
}

impl PhysicalLayout {
    pub fn from_layout_type(kind: u32) -> Self {
        match kind {
            0x4953_4F20 => Self::Iso,
            0x4A49_5320 => Self::Jis,
            _ => Self::Ansi,
        }
    }
}

const ISO_SWAP: [(u16, u16); 2] = [(0x35, ISO_SECTION), (0x64, ANSI_GRAVE)];

const HID_TO_KEYCODE: &[(u16, u16)] = &[
    (0x04, ANSI_A), (0x05, ANSI_B), (0x06, ANSI_C), (0x07, ANSI_D),
    (0x08, ANSI_E), (0x09, ANSI_F), (0x0A, ANSI_G), (0x0B, ANSI_H),
    (0x0C, ANSI_I), (0x0D, ANSI_J), (0x0E, ANSI_K), (0x0F, ANSI_L),
    (0x10, ANSI_M), (0x11, ANSI_N), (0x12, ANSI_O), (0x13, ANSI_P),
    (0x14, ANSI_Q), (0x15, ANSI_R), (0x16, ANSI_S), (0x17, ANSI_T),
    (0x18, ANSI_U), (0x19, ANSI_V), (0x1A, ANSI_W), (0x1B, ANSI_X),
    (0x1C, ANSI_Y), (0x1D, ANSI_Z),
    (0x1E, ANSI_1), (0x1F, ANSI_2), (0x20, ANSI_3), (0x21, ANSI_4),
    (0x22, ANSI_5), (0x23, ANSI_6), (0x24, ANSI_7), (0x25, ANSI_8),
    (0x26, ANSI_9), (0x27, ANSI_0),
    (0x28, RETURN), (0x29, ESCAPE), (0x2A, DELETE), (0x2B, TAB), (0x2C, SPACE),
    (0x2D, ANSI_MINUS), (0x2E, ANSI_EQUAL), (0x2F, ANSI_LEFT_BRACKET),
    (0x30, ANSI_RIGHT_BRACKET), (0x31, ANSI_BACKSLASH), (0x32, ANSI_BACKSLASH),
    (0x33, ANSI_SEMICOLON), (0x34, ANSI_QUOTE), (0x35, ANSI_GRAVE),
    (0x36, ANSI_COMMA), (0x37, ANSI_PERIOD), (0x38, ANSI_SLASH),
    (0x39, 0x39),
    (0x3A, F1), (0x3B, F2), (0x3C, F3), (0x3D, F4), (0x3E, F5), (0x3F, F6),
    (0x40, F7), (0x41, F8), (0x42, F9), (0x43, F10), (0x44, F11), (0x45, F12),
    (0x68, 0x69), (0x69, 0x6B), (0x6A, 0x71),
    (0x6B, 0x6A), (0x6C, 0x40), (0x6D, 0x4F), (0x6E, 0x50), (0x6F, 0x5A),
    (0x46, 0x69), (0x47, 0x6B), (0x48, 0x71),
    (0x49, 0x72), (0x4A, HOME), (0x4B, PAGE_UP), (0x4C, FORWARD_DELETE),
    (0x4D, END), (0x4E, PAGE_DOWN),
    (0x4F, RIGHT_ARROW), (0x50, LEFT_ARROW), (0x51, DOWN_ARROW), (0x52, UP_ARROW),
    (0x53, 0x47), (0x54, 0x4B), (0x55, 0x43), (0x56, 0x4E), (0x57, 0x45),
    (0x58, 0x4C), (0x59, 0x53), (0x5A, 0x54), (0x5B, 0x55), (0x5C, 0x56),
    (0x5D, 0x57), (0x5E, 0x58), (0x5F, 0x59), (0x60, 0x5B), (0x61, 0x5C),
    (0x62, 0x52), (0x63, 0x41), (0x64, ISO_SECTION), (0x65, 0x6E), (0x67, 0x51),
    (0x7F, 0x4A), (0x80, 0x48), (0x81, 0x49),
    (0x87, 0x5E), (0x89, 0x5D), (0x90, 0x68), (0x91, 0x66),
    (0xE0, 0x3B), (0xE1, 0x38), (0xE2, 0x3A), (0xE3, 0x37),
    (0xE4, 0x3E), (0xE5, 0x3C), (0xE6, 0x3D), (0xE7, 0x36),
];

pub fn from_hid_usage(usage: u16, layout: PhysicalLayout) -> Option<u16> {
    if layout == PhysicalLayout::Iso {
        if let Some(&(_, code)) = ISO_SWAP.iter().find(|(from, _)| *from == usage) {
            return Some(code);
        }
    }
    HID_TO_KEYCODE
        .iter()
        .find(|(from, _)| *from == usage)
        .map(|&(_, code)| code)
}

pub fn to_hid_usage(code: u16, layout: PhysicalLayout) -> Option<u16> {
    if layout == PhysicalLayout::Iso {
        if let Some(&(usage, _)) = ISO_SWAP.iter().find(|(_, to)| *to == code) {
            return Some(usage);
        }
    }
    HID_TO_KEYCODE
        .iter()
        .find(|(_, to)| *to == code)
        .map(|&(usage, _)| usage)
}
```

Add `#[rustfmt::skip]` above `const HID_TO_KEYCODE` so the rows stay grouped.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p qol-hotkeys`
Expected: PASS. If `every_named_keycode_survives_a_trip_through_hid` names a key, add that key's usage to the table; do not skip it.

- [ ] **Step 5: Commit**

```bash
git add libs/hotkeys/src/keycode/backends/carbon.rs
git commit -m "feat(hotkeys): map HID usages to macOS keycodes"
```

---

### Task 3: Shared keyboard layout translation

**Files:**
- Create: `libs/hotkeys/src/layout/mod.rs`
- Create: `libs/hotkeys/src/layout/platform/mod.rs`, `libs/hotkeys/src/layout/platform/macos.rs`, `libs/hotkeys/src/layout/platform/fallback.rs`
- Create: `libs/hotkeys/tests/macos_layout.rs`
- Modify: `libs/hotkeys/src/lib.rs`, `libs/hotkeys/Cargo.toml`
- Modify: `apps/tray/src/hotkeys/capture/platform/macos/layout.rs`

**Interfaces:**
- Consumes: `PhysicalLayout` (Task 2).
- Produces: `qol_hotkeys::layout::{KeyLayout, LayoutError, SHIFT_STATE, OPTION_STATE, physical_layout}`.
  - `KeyLayout::current() -> Result<KeyLayout, LayoutError>` (main thread only)
  - `KeyLayout::by_id(id: &str) -> Result<KeyLayout, LayoutError>` (main thread only)
  - `KeyLayout::id(&self) -> &str`
  - `KeyLayout::translate(&self, code: u16, modifier_state: u32, dead_key_state: &mut u32) -> Option<String>`; `None` when `UCKeyTranslate` fails, `Some("")` for a dead key.
  - `current_layout_id() -> Result<String, LayoutError>` (main thread only, cheap)
  - `physical_layout() -> PhysicalLayout` (main thread only)

- [ ] **Step 1: Write the failing test**

`libs/hotkeys/tests/macos_layout.rs`. It runs without the test harness so it runs on the main thread, which TIS requires. The expected values come from a probe of this Mac's installed layouts on 2026-09-30.

```rust
#[cfg(target_os = "macos")]
fn main() {
    use qol_hotkeys::layout::{KeyLayout, OPTION_STATE, SHIFT_STATE};

    let danish = KeyLayout::by_id("com.apple.keylayout.Danish").expect("Danish layout");
    let us = KeyLayout::by_id("com.apple.keylayout.US").expect("US layout");
    let typed = |layout: &KeyLayout, code: u16, state: u32| {
        let mut dead = 0;
        layout.translate(code, state, &mut dead)
    };

    assert_eq!(danish.id(), "com.apple.keylayout.Danish");
    assert_eq!(typed(&danish, 0x2A, OPTION_STATE).as_deref(), Some("@"));
    assert_eq!(typed(&danish, 0x15, SHIFT_STATE).as_deref(), Some("€"));
    assert_eq!(typed(&danish, 0x0A, 0).as_deref(), Some("$"));
    assert_eq!(typed(&us, 0x13, SHIFT_STATE).as_deref(), Some("@"));

    let mut dead = 0;
    assert_eq!(danish.translate(0x1E, OPTION_STATE, &mut dead).as_deref(), Some(""));
    assert_ne!(dead, 0, "option+¨ is a dead key on Danish");
    assert_eq!(danish.translate(0x31, 0, &mut dead).as_deref(), Some("~"));

    assert!(KeyLayout::by_id("com.example.no-such-layout").is_err());
    println!("macos_layout: ok");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
```

Add to `libs/hotkeys/Cargo.toml`:

```toml
[target.'cfg(target_os = "macos")'.dependencies]
core-foundation = "0.10"

[[test]]
name = "macos_layout"
harness = false
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p qol-hotkeys --test macos_layout`
Expected: compile error, `could not find layout in qol_hotkeys`.

- [ ] **Step 3: Implement**

`libs/hotkeys/src/lib.rs` gains `pub mod layout;`.

`libs/hotkeys/src/layout/mod.rs`:

```rust
mod platform;

pub use platform::{current_layout_id, physical_layout, KeyLayout};

pub const SHIFT_STATE: u32 = 0x02;
pub const OPTION_STATE: u32 = 0x08;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutError {
    NoInputSource,
    NotFound(String),
    NoKeyMap(String),
    Unsupported,
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoInputSource => write!(formatter, "no keyboard input source is selected"),
            Self::NotFound(id) => write!(formatter, "keyboard layout {id} is not installed"),
            Self::NoKeyMap(id) => write!(formatter, "keyboard layout {id} has no key map"),
            Self::Unsupported => write!(formatter, "keyboard layouts are only readable on macOS"),
        }
    }
}

impl std::error::Error for LayoutError {}
```

`libs/hotkeys/src/layout/platform/mod.rs`:

```rust
#[cfg(not(target_os = "macos"))]
mod fallback;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(target_os = "macos"))]
pub use fallback::{current_layout_id, physical_layout, KeyLayout};
#[cfg(target_os = "macos")]
pub use macos::{current_layout_id, physical_layout, KeyLayout};
```

`libs/hotkeys/src/layout/platform/fallback.rs`:

```rust
use crate::keycode::macos_keycode::PhysicalLayout;
use crate::layout::LayoutError;

pub struct KeyLayout {
    id: String,
}

impl KeyLayout {
    pub fn current() -> Result<Self, LayoutError> {
        Err(LayoutError::Unsupported)
    }

    pub fn by_id(_id: &str) -> Result<Self, LayoutError> {
        Err(LayoutError::Unsupported)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn translate(&self, _code: u16, _state: u32, _dead_key_state: &mut u32) -> Option<String> {
        None
    }
}

pub fn current_layout_id() -> Result<String, LayoutError> {
    Err(LayoutError::Unsupported)
}

pub fn physical_layout() -> PhysicalLayout {
    PhysicalLayout::Ansi
}
```

`libs/hotkeys/src/layout/platform/macos.rs`:

```rust
use std::ffi::c_void;

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, TCFType};
use core_foundation::data::{CFData, CFDataRef};
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::{CFString, CFStringRef};

use crate::keycode::macos_keycode::PhysicalLayout;
use crate::layout::LayoutError;

const KEY_ACTION_DOWN: u16 = 0;

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    static kTISPropertyUnicodeKeyLayoutData: CFStringRef;
    static kTISPropertyInputSourceID: CFStringRef;
    fn TISCopyCurrentKeyboardLayoutInputSource() -> *const c_void;
    fn TISCreateInputSourceList(properties: *const c_void, include_all: u8) -> CFArrayRef;
    fn TISGetInputSourceProperty(source: *const c_void, key: CFStringRef) -> *const c_void;
    fn LMGetKbdType() -> u8;
    fn KBGetLayoutType(keyboard_type: i16) -> u32;
    #[allow(clippy::too_many_arguments)]
    fn UCKeyTranslate(
        layout: *const c_void,
        code: u16,
        action: u16,
        modifier_state: u32,
        keyboard_type: u32,
        options: u32,
        dead_key_state: *mut u32,
        max_length: usize,
        actual_length: *mut usize,
        chars: *mut u16,
    ) -> i32;
}

pub struct KeyLayout {
    id: String,
    data: CFData,
    keyboard_type: u32,
}

impl KeyLayout {
    pub fn current() -> Result<Self, LayoutError> {
        let source = unsafe { TISCopyCurrentKeyboardLayoutInputSource() };
        if source.is_null() {
            return Err(LayoutError::NoInputSource);
        }
        Self::from_source(unsafe { CFType::wrap_under_create_rule(source) })
    }

    pub fn by_id(id: &str) -> Result<Self, LayoutError> {
        let key = unsafe { CFString::wrap_under_get_rule(kTISPropertyInputSourceID) };
        let filter =
            CFDictionary::from_CFType_pairs(&[(key.as_CFType(), CFString::new(id).as_CFType())]);
        let list = unsafe { TISCreateInputSourceList(filter.as_CFTypeRef(), 1) };
        if list.is_null() {
            return Err(LayoutError::NotFound(id.to_string()));
        }
        let list: CFArray<CFType> = unsafe { CFArray::wrap_under_create_rule(list) };
        let source = list
            .get(0)
            .map(|source| source.clone())
            .ok_or_else(|| LayoutError::NotFound(id.to_string()))?;
        Self::from_source(source)
    }

    fn from_source(source: CFType) -> Result<Self, LayoutError> {
        let id = source_id(&source);
        let data = unsafe {
            TISGetInputSourceProperty(source.as_CFTypeRef(), kTISPropertyUnicodeKeyLayoutData)
        };
        if data.is_null() {
            return Err(LayoutError::NoKeyMap(id));
        }
        Ok(Self {
            id,
            data: unsafe { CFData::wrap_under_get_rule(data as CFDataRef) },
            keyboard_type: u32::from(unsafe { LMGetKbdType() }),
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn translate(&self, code: u16, modifier_state: u32, dead_key_state: &mut u32) -> Option<String> {
        let mut length = 0;
        let mut chars = [0u16; 4];
        let status = unsafe {
            UCKeyTranslate(
                self.data.bytes().as_ptr().cast(),
                code,
                KEY_ACTION_DOWN,
                modifier_state,
                self.keyboard_type,
                0,
                dead_key_state,
                chars.len(),
                &mut length,
                chars.as_mut_ptr(),
            )
        };
        (status == 0).then(|| String::from_utf16_lossy(&chars[..length]))
    }
}

pub fn current_layout_id() -> Result<String, LayoutError> {
    let source = unsafe { TISCopyCurrentKeyboardLayoutInputSource() };
    if source.is_null() {
        return Err(LayoutError::NoInputSource);
    }
    Ok(source_id(&unsafe { CFType::wrap_under_create_rule(source) }))
}

pub fn physical_layout() -> PhysicalLayout {
    let keyboard_type = unsafe { LMGetKbdType() };
    PhysicalLayout::from_layout_type(unsafe { KBGetLayoutType(i16::from(keyboard_type)) })
}

fn source_id(source: &CFType) -> String {
    let id = unsafe { TISGetInputSourceProperty(source.as_CFTypeRef(), kTISPropertyInputSourceID) };
    if id.is_null() {
        return String::new();
    }
    unsafe { CFString::wrap_under_get_rule(id as CFStringRef) }.to_string()
}
```

- [ ] **Step 4: Move the tray onto the shared layout**

In `apps/tray/src/hotkeys/capture/platform/macos/layout.rs` delete the `#[link(name = "Carbon"...)] extern "C"` block, `KEY_ACTION_DOWN`, `SHIFT_STATE`, `read_current_layout` and `translate`, and the `core_foundation` and `c_void` imports. Keep `LayoutSymbols`, `from_translation`, `ansi`, `ansi_symbol` and the tests. Replace the top of the file and `current()` with:

```rust
use qol_hotkeys::layout::{KeyLayout, SHIFT_STATE};
use qol_hotkeys::macos_keycode;
use std::collections::HashMap;
use std::sync::OnceLock;

const KEYCODES: std::ops::Range<u16> = 0..0x80;
const LEVELS: [u32; 2] = [0, SHIFT_STATE];
```

```rust
    pub(super) fn current() -> &'static Self {
        static CURRENT: OnceLock<LayoutSymbols> = OnceLock::new();
        CURRENT.get_or_init(|| match KeyLayout::current() {
            Ok(layout) => Self::from_translation(|code, level| single_symbol(&layout, code, level)),
            Err(error) => {
                log::warn!("[hotkeys] {error}; using US positions");
                Self::ansi()
            }
        })
    }
```

and add below `ansi_symbol`:

```rust
fn single_symbol(layout: &KeyLayout, code: u16, level: u32) -> Option<char> {
    let mut dead_key_state = 0;
    let text = layout.translate(code, level, &mut dead_key_state)?;
    let mut chars = text.chars();
    let symbol = chars.next()?;
    (chars.next().is_none() && !symbol.is_control()).then_some(symbol)
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p qol-hotkeys && cargo test -p qol-tray hotkeys::capture && cargo clippy -p qol-hotkeys -p qol-tray --all-targets -- -D warnings`
Expected: `macos_layout: ok` printed, all PASS, no clippy findings. Also run `cargo check -p qol-hotkeys --target x86_64-unknown-linux-gnu` to prove the fallback compiles, with the `ring` caveat from Global Constraints.

- [ ] **Step 6: Commit**

```bash
git add libs/hotkeys apps/tray/src/hotkeys/capture/platform/macos/layout.rs Cargo.lock libs/workspace-hack
git commit -m "refactor(hotkeys): share the keyboard layout translation with the tray"
```

---

### Task 4: Characters to key strokes

**Files:**
- Create: `plugins/keyremap/src/platform/macos/layout/mod.rs`
- Modify: `plugins/keyremap/src/platform/macos/mod.rs` (add `mod layout;`)

**Interfaces:**
- Consumes: `qol_hotkeys::layout::{KeyLayout, SHIFT_STATE, OPTION_STATE, current_layout_id, physical_layout}`, `PhysicalLayout` (Tasks 2, 3).
- Produces:
  - `KeyStroke { keycode: u16, shift: bool, option: bool }` (`Copy`, `PartialEq`, `Debug`)
  - `CharTable::from_translation(impl Fn(u16, u32, &mut u32) -> Option<String>) -> CharTable`
  - `CharTable::sequence_for(&self, text: &str) -> Option<Vec<KeyStroke>>`
  - `CharTable::char_at(&self, keycode: u16, shift: bool, option: bool) -> Option<&str>`
  - `LayoutSnapshot { id: String, table: CharTable, physical: PhysicalLayout }`, `LayoutSnapshot::empty()`, `LayoutSnapshot::read_current() -> Result<LayoutSnapshot, LayoutError>` (main thread only)
  - `LayoutStore::new(LayoutSnapshot)`, `get(&self) -> Arc<LayoutSnapshot>`, `refresh(&self)` (main thread only)
  - `missing_characters<'a>(table: &CharTable, texts: impl IntoIterator<Item = &'a str>) -> Vec<String>`

- [ ] **Step 1: Write the failing tests**

`plugins/keyremap/src/platform/macos/layout/mod.rs` test module. The Danish table below is the probe output for the 12 characters the current rules emit.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const DEAD_TILDE: u32 = 1;
    const DEAD_ACUTE: u32 = 2;

    fn danish(code: u16, state: u32, dead: &mut u32) -> Option<String> {
        if code == SPACE && state == 0 {
            let text = match std::mem::take(dead) {
                DEAD_TILDE => "~",
                DEAD_ACUTE => "´",
                _ => " ",
            };
            return Some(text.to_string());
        }
        let text = match (code, state) {
            (0x0A, 0) => "$",
            (0x2A, OPTION_STATE) => "@",
            (0x1C, OPTION_STATE) => "[",
            (0x1A, s) if s == SHIFT_STATE | OPTION_STATE => "\\",
            (0x19, OPTION_STATE) => "]",
            (0x1C, s) if s == SHIFT_STATE | OPTION_STATE => "{",
            (0x22, OPTION_STATE) => "|",
            (0x19, s) if s == SHIFT_STATE | OPTION_STATE => "}",
            (0x15, OPTION_STATE) => "£",
            (0x2E, OPTION_STATE) => "µ",
            (0x15, SHIFT_STATE) => "€",
            (0x00, 0) => "a",
            (0x1E, OPTION_STATE) => {
                *dead = DEAD_TILDE;
                ""
            }
            (0x18, 0) => {
                *dead = DEAD_ACUTE;
                ""
            }
            _ => return None,
        };
        Some(text.to_string())
    }

    fn stroke(keycode: u16, shift: bool, option: bool) -> KeyStroke {
        KeyStroke { keycode, shift, option }
    }

    #[test]
    fn every_current_rule_character_has_a_danish_sequence() {
        let table = CharTable::from_translation(danish);
        for text in ["$", "@", "[", "\\", "]", "{", "|", "}", "~", "£", "µ", "€"] {
            assert!(table.sequence_for(text).is_some(), "{text}");
        }
        assert_eq!(table.sequence_for("@"), Some(vec![stroke(0x2A, false, true)]));
        assert_eq!(table.sequence_for("€"), Some(vec![stroke(0x15, true, false)]));
        assert_eq!(table.sequence_for("$"), Some(vec![stroke(0x0A, false, false)]));
        assert_eq!(table.sequence_for("{"), Some(vec![stroke(0x1C, true, true)]));
    }

    #[test]
    fn a_dead_key_character_is_the_dead_key_then_space() {
        let table = CharTable::from_translation(danish);
        assert_eq!(
            table.sequence_for("~"),
            Some(vec![stroke(0x1E, false, true), stroke(SPACE, false, false)])
        );
        assert_eq!(
            table.sequence_for("´"),
            Some(vec![stroke(0x18, false, false), stroke(SPACE, false, false)])
        );
    }

    #[test]
    fn a_character_the_layout_cannot_type_has_no_sequence() {
        let table = CharTable::from_translation(danish);
        assert_eq!(table.sequence_for("あ"), None);
        assert_eq!(table.sequence_for("a あ"), None);
        assert_eq!(missing_characters(&table, ["@", "あ", "✓", "あ"]), vec!["あ", "✓"]);
    }

    #[test]
    fn multi_character_text_concatenates_sequences() {
        let table = CharTable::from_translation(danish);
        assert_eq!(
            table.sequence_for("a@"),
            Some(vec![stroke(0x00, false, false), stroke(0x2A, false, true)])
        );
    }

    #[test]
    fn char_at_reports_what_a_key_types() {
        let table = CharTable::from_translation(danish);
        assert_eq!(table.char_at(0x2A, false, true), Some("@"));
        assert_eq!(table.char_at(0x15, true, false), Some("€"));
        assert_eq!(table.char_at(0x1E, false, true), None);
        assert_eq!(table.char_at(0x7F, false, false), None);
    }

    #[test]
    fn store_serves_readers_without_touching_tis() {
        let store = LayoutStore::new(LayoutSnapshot {
            id: "test.danish".to_string(),
            table: CharTable::from_translation(danish),
            physical: PhysicalLayout::Iso,
        });
        let reader = std::thread::spawn({
            let store = std::sync::Arc::new(store);
            move || store.get().table.sequence_for("@")
        });
        assert_eq!(reader.join().unwrap(), Some(vec![stroke(0x2A, false, true)]));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p qol-keyremap layout::tests`
Expected: compile error, `cannot find type CharTable`.

- [ ] **Step 3: Implement**

Top of `layout/mod.rs`:

```rust
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};

use qol_hotkeys::layout::{self as keyboard, KeyLayout, LayoutError, OPTION_STATE, SHIFT_STATE};
use qol_hotkeys::macos_keycode::{PhysicalLayout, SPACE};

const KEYCODES: std::ops::Range<u16> = 0..0x80;
const STATES: [(bool, bool); 4] = [(false, false), (true, false), (false, true), (true, true)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyStroke {
    pub(crate) keycode: u16,
    pub(crate) shift: bool,
    pub(crate) option: bool,
}

impl KeyStroke {
    fn modifier_state(self) -> u32 {
        let shift = if self.shift { SHIFT_STATE } else { 0 };
        let option = if self.option { OPTION_STATE } else { 0 };
        shift | option
    }
}

pub(crate) struct CharTable {
    direct: HashMap<char, KeyStroke>,
    dead: HashMap<char, KeyStroke>,
    typed: HashMap<(u16, bool, bool), String>,
}

impl CharTable {
    pub(crate) fn from_translation(translate: impl Fn(u16, u32, &mut u32) -> Option<String>) -> Self {
        let mut direct = HashMap::new();
        let mut dead = HashMap::new();
        let mut typed = HashMap::new();
        for (shift, option) in STATES {
            for keycode in KEYCODES {
                let stroke = KeyStroke { keycode, shift, option };
                let mut dead_state = 0;
                let Some(text) = translate(keycode, stroke.modifier_state(), &mut dead_state) else {
                    continue;
                };
                if dead_state != 0 {
                    if let Some(composed) = translate(SPACE, 0, &mut dead_state) {
                        if let Some(symbol) = single_char(&composed) {
                            dead.entry(symbol).or_insert(stroke);
                        }
                    }
                    continue;
                }
                if let Some(symbol) = single_char(&text) {
                    direct.entry(symbol).or_insert(stroke);
                }
                if !text.is_empty() {
                    typed.insert((keycode, shift, option), text);
                }
            }
        }
        Self { direct, dead, typed }
    }

    pub(crate) fn sequence_for(&self, text: &str) -> Option<Vec<KeyStroke>> {
        let mut strokes = Vec::new();
        for symbol in text.chars() {
            if let Some(stroke) = self.direct.get(&symbol) {
                strokes.push(*stroke);
                continue;
            }
            let dead = self.dead.get(&symbol)?;
            strokes.push(*dead);
            strokes.push(KeyStroke { keycode: SPACE, shift: false, option: false });
        }
        (!strokes.is_empty()).then_some(strokes)
    }

    pub(crate) fn char_at(&self, keycode: u16, shift: bool, option: bool) -> Option<&str> {
        self.typed.get(&(keycode, shift, option)).map(String::as_str)
    }
}

fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let symbol = chars.next()?;
    (chars.next().is_none() && !symbol.is_control()).then_some(symbol)
}

pub(crate) fn missing_characters<'a>(
    table: &CharTable,
    texts: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let mut missing = BTreeSet::new();
    let mut ordered = Vec::new();
    for text in texts {
        for symbol in text.chars() {
            let one = symbol.to_string();
            if table.sequence_for(&one).is_none() && missing.insert(symbol) {
                ordered.push(one);
            }
        }
    }
    ordered
}

pub(crate) struct LayoutSnapshot {
    pub(crate) id: String,
    pub(crate) table: CharTable,
    pub(crate) physical: PhysicalLayout,
}

impl LayoutSnapshot {
    pub(crate) fn empty() -> Self {
        Self {
            id: String::new(),
            table: CharTable::from_translation(|_, _, _| None),
            physical: PhysicalLayout::Ansi,
        }
    }

    pub(crate) fn read_current() -> Result<Self, LayoutError> {
        let layout = KeyLayout::current()?;
        Ok(Self {
            id: layout.id().to_string(),
            table: CharTable::from_translation(|code, state, dead| layout.translate(code, state, dead)),
            physical: keyboard::physical_layout(),
        })
    }
}

pub(crate) struct LayoutStore {
    current: RwLock<Arc<LayoutSnapshot>>,
}

impl LayoutStore {
    pub(crate) fn new(snapshot: LayoutSnapshot) -> Self {
        Self { current: RwLock::new(Arc::new(snapshot)) }
    }

    pub(crate) fn get(&self) -> Arc<LayoutSnapshot> {
        self.current
            .read()
            .map(|guard| Arc::clone(&guard))
            .unwrap_or_else(|poisoned| Arc::clone(&poisoned.into_inner()))
    }

    pub(crate) fn refresh(&self) {
        let physical = keyboard::physical_layout();
        let current = self.get();
        let same_source = keyboard::current_layout_id().is_ok_and(|id| id == current.id);
        if same_source && physical == current.physical {
            return;
        }
        match LayoutSnapshot::read_current() {
            Ok(snapshot) => {
                log::info!("keyboard layout is now {} ({:?})", snapshot.id, snapshot.physical);
                if let Ok(mut guard) = self.current.write() {
                    *guard = Arc::new(snapshot);
                }
            }
            Err(error) => log::warn!("keeping the previous keyboard layout: {error}"),
        }
    }
}
```

`missing_characters` returns characters in first-seen order with duplicates removed; the `BTreeSet` is only the seen set. Add `#[allow(dead_code)] mod layout;` to `plugins/keyremap/src/platform/macos/mod.rs`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p qol-keyremap layout::tests`
Expected: PASS, 6 tests.

- [ ] **Step 5: Commit**

```bash
git add plugins/keyremap/src/platform/macos/layout plugins/keyremap/src/platform/macos/mod.rs
git commit -m "feat(keyremap): turn characters into key strokes for the active layout"
```

---

### Task 5: pqrs frame and request codec

**Files:**
- Create: `plugins/keyremap/src/platform/macos/virtual_hid/mod.rs`
- Create: `plugins/keyremap/src/platform/macos/virtual_hid/client/mod.rs` (only `pub(crate) mod frame; pub(crate) mod request;` in this task)
- Create: `plugins/keyremap/src/platform/macos/virtual_hid/client/frame.rs`
- Create: `plugins/keyremap/src/platform/macos/virtual_hid/client/request.rs`
- Modify: `plugins/keyremap/src/platform/macos/mod.rs` (add `mod virtual_hid;`)

**Interfaces:**
- Produces:
  - `virtual_hid::{PQRS_SOCKET, PQRS_DAEMON_BINARY, PQRS_MANAGER_BINARY, PQRS_DAEMON_LABEL, OWN_DAEMON_LABEL}` (`&str` constants, values from Global Constraints)
  - `frame::Frame { Heartbeat, UserData(Vec<u8>), HealthCheck, HealthCheckResponse, Request { id: u64, payload: Vec<u8> }, Response { id: u64, payload: Vec<u8> } }`, `Frame::encode(&self) -> Vec<u8>`
  - `frame::Decoder::default()`, `push(&mut self, &[u8])`, `next_frame(&mut self) -> Result<Option<Frame>, FrameError>`
  - `frame::FrameError { Empty, Oversized(usize), UnknownType(u8), MissingRequestId }`
  - `request::Keys = [u16; 32]`
  - `request::Request { KeyboardInitialize { country_code: u64 }, KeyboardReset, Keyboard { modifiers: u8, keys: Keys }, Consumer(Keys), AppleVendorKeyboard(Keys), AppleVendorTopCase(Keys) }`, `Request::encode(&self) -> Vec<u8>`
  - `request::Status { DriverActivated(bool), DriverConnected(bool), DriverVersionMismatched(bool), KeyboardReady(bool) }` (pointing-device status is skipped; keyremap never creates the virtual pointer)
  - `request::parse_status(&[u8]) -> Result<Vec<Status>, StatusError>`, `StatusError { OddLength(usize), UnknownResponse(u8) }`

The wire format comes from the pqrs headers (`pqrs/unix_domain_stream/impl/protocol.hpp`, `virtual_hid_device_service/{request,response,client}.hpp`): a u32 big-endian body length, then a type byte, then for request and response frames a u64 big-endian request id. Request payloads start with the client protocol version as u16 little-endian and a request byte; report structs are packed and little-endian.

- [ ] **Step 1: Write the failing tests**

`frame.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_is_a_one_byte_body() {
        assert_eq!(Frame::Heartbeat.encode(), vec![0, 0, 0, 1, 0]);
        assert_eq!(Frame::HealthCheckResponse.encode(), vec![0, 0, 0, 1, 3]);
    }

    #[test]
    fn request_frames_carry_a_big_endian_id() {
        let frame = Frame::Request { id: 0x0102, payload: vec![7, 0, 2] };
        assert_eq!(
            frame.encode(),
            vec![0, 0, 0, 12, 4, 0, 0, 0, 0, 0, 0, 1, 2, 7, 0, 2]
        );
    }

    #[test]
    fn every_frame_round_trips() {
        let frames = [
            Frame::Heartbeat,
            Frame::UserData(vec![1, 2, 3]),
            Frame::HealthCheck,
            Frame::HealthCheckResponse,
            Frame::Request { id: 9, payload: vec![4, 1] },
            Frame::Response { id: u64::MAX, payload: Vec::new() },
        ];
        let mut decoder = Decoder::default();
        for frame in &frames {
            decoder.push(&frame.encode());
        }
        for frame in frames {
            assert_eq!(decoder.next_frame(), Ok(Some(frame)));
        }
        assert_eq!(decoder.next_frame(), Ok(None));
    }

    #[test]
    fn a_frame_split_across_reads_decodes_once_complete() {
        let bytes = Frame::Response { id: 3, payload: vec![4, 1, 2, 1] }.encode();
        let mut decoder = Decoder::default();
        for byte in &bytes[..bytes.len() - 1] {
            decoder.push(&[*byte]);
            assert_eq!(decoder.next_frame(), Ok(None));
        }
        decoder.push(&bytes[bytes.len() - 1..]);
        assert_eq!(
            decoder.next_frame(),
            Ok(Some(Frame::Response { id: 3, payload: vec![4, 1, 2, 1] }))
        );
    }

    #[test]
    fn oversized_empty_and_unknown_frames_are_errors() {
        let mut decoder = Decoder::default();
        decoder.push(&[0, 0, 4, 1]);
        assert_eq!(decoder.next_frame(), Err(FrameError::Oversized(1025)));

        let mut decoder = Decoder::default();
        decoder.push(&[0, 0, 0, 0]);
        assert_eq!(decoder.next_frame(), Err(FrameError::Empty));

        let mut decoder = Decoder::default();
        decoder.push(&[0, 0, 0, 1, 9]);
        assert_eq!(decoder.next_frame(), Err(FrameError::UnknownType(9)));

        let mut decoder = Decoder::default();
        decoder.push(&[0, 0, 0, 3, 5, 0, 0]);
        assert_eq!(decoder.next_frame(), Err(FrameError::MissingRequestId));
    }
}
```

`request.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_initialize_sends_vendor_product_and_country_as_u64() {
        let bytes = Request::KeyboardInitialize { country_code: 0x0D }.encode();
        let mut expected = vec![7, 0, 0];
        expected.extend(0x16c0u64.to_le_bytes());
        expected.extend(0x27dbu64.to_le_bytes());
        expected.extend(0x0Du64.to_le_bytes());
        assert_eq!(bytes, expected);
    }

    #[test]
    fn keyboard_report_is_id_modifiers_reserved_then_32_keys() {
        let mut keys = [0u16; 32];
        keys[0] = 0x04;
        keys[31] = 0x0102;
        let bytes = Request::Keyboard { modifiers: 0x08, keys }.encode();
        assert_eq!(bytes.len(), 3 + 67);
        assert_eq!(&bytes[..6], &[7, 0, 6, 1, 0x08, 0]);
        assert_eq!(&bytes[6..8], &[0x04, 0]);
        assert_eq!(&bytes[68..70], &[0x02, 0x01]);
    }

    #[test]
    fn page_reports_carry_their_report_ids() {
        let keys = [0u16; 32];
        assert_eq!(&Request::Consumer(keys).encode()[..4], &[7, 0, 7, 2]);
        assert_eq!(&Request::AppleVendorKeyboard(keys).encode()[..4], &[7, 0, 8, 4]);
        assert_eq!(&Request::AppleVendorTopCase(keys).encode()[..4], &[7, 0, 9, 3]);
        assert_eq!(Request::Consumer(keys).encode().len(), 3 + 65);
        assert_eq!(Request::KeyboardReset.encode(), vec![7, 0, 2]);
    }

    #[test]
    fn status_pairs_parse_and_none_is_skipped() {
        assert_eq!(
            parse_status(&[1, 1, 2, 0, 0, 0, 4, 1]),
            Ok(vec![
                Status::DriverActivated(true),
                Status::DriverConnected(false),
                Status::KeyboardReady(true),
            ])
        );
        assert_eq!(parse_status(&[]), Ok(Vec::new()));
        assert_eq!(parse_status(&[4]), Err(StatusError::OddLength(1)));
        assert_eq!(parse_status(&[9, 1]), Err(StatusError::UnknownResponse(9)));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p qol-keyremap virtual_hid::client`
Expected: compile error, `cannot find type Frame`.

- [ ] **Step 3: Implement**

`virtual_hid/mod.rs`:

```rust
pub(crate) mod client;

pub(crate) const PQRS_SOCKET: &str =
    "/Library/Application Support/org.pqrs/tmp/rootonly/karabiner_virtual_hid_device_service.sock";
pub(crate) const PQRS_DAEMON_BINARY: &str = "/Library/Application Support/org.pqrs/Karabiner-DriverKit-VirtualHIDDevice/Applications/Karabiner-VirtualHIDDevice-Daemon.app/Contents/MacOS/Karabiner-VirtualHIDDevice-Daemon";
pub(crate) const PQRS_MANAGER_BINARY: &str =
    "/Applications/.Karabiner-VirtualHIDDevice-Manager.app/Contents/MacOS/Karabiner-VirtualHIDDevice-Manager";
pub(crate) const PQRS_DAEMON_LABEL: &str = "org.pqrs.service.daemon.Karabiner-VirtualHIDDevice-Daemon";
pub(crate) const OWN_DAEMON_LABEL: &str = "com.qol-tools.keyremap.vhid-daemon";
```

`client/frame.rs`:

```rust
pub(crate) const MAX_BODY_SIZE: usize = 1024;
const HEADER_SIZE: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Frame {
    Heartbeat,
    UserData(Vec<u8>),
    HealthCheck,
    HealthCheckResponse,
    Request { id: u64, payload: Vec<u8> },
    Response { id: u64, payload: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FrameError {
    Empty,
    Oversized(usize),
    UnknownType(u8),
    MissingRequestId,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(formatter, "empty frame"),
            Self::Oversized(length) => write!(formatter, "frame body of {length} bytes is over {MAX_BODY_SIZE}"),
            Self::UnknownType(kind) => write!(formatter, "unknown frame type {kind}"),
            Self::MissingRequestId => write!(formatter, "request frame without a request id"),
        }
    }
}

impl Frame {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let (kind, id, payload): (u8, Option<u64>, &[u8]) = match self {
            Self::Heartbeat => (0, None, &[]),
            Self::UserData(payload) => (1, None, payload),
            Self::HealthCheck => (2, None, &[]),
            Self::HealthCheckResponse => (3, None, &[]),
            Self::Request { id, payload } => (4, Some(*id), payload),
            Self::Response { id, payload } => (5, Some(*id), payload),
        };
        let body_len = 1 + id.map_or(0, |_| 8) + payload.len();
        let mut bytes = Vec::with_capacity(HEADER_SIZE + body_len);
        bytes.extend_from_slice(&(body_len as u32).to_be_bytes());
        bytes.push(kind);
        if let Some(id) = id {
            bytes.extend_from_slice(&id.to_be_bytes());
        }
        bytes.extend_from_slice(payload);
        bytes
    }

    fn decode_body(body: &[u8]) -> Result<Self, FrameError> {
        let (&kind, rest) = body.split_first().ok_or(FrameError::Empty)?;
        let with_id = |rest: &[u8]| {
            let (id, payload) = rest
                .split_first_chunk::<8>()
                .ok_or(FrameError::MissingRequestId)?;
            Ok::<_, FrameError>((u64::from_be_bytes(*id), payload.to_vec()))
        };
        match kind {
            0 => Ok(Self::Heartbeat),
            1 => Ok(Self::UserData(rest.to_vec())),
            2 => Ok(Self::HealthCheck),
            3 => Ok(Self::HealthCheckResponse),
            4 => with_id(rest).map(|(id, payload)| Self::Request { id, payload }),
            5 => with_id(rest).map(|(id, payload)| Self::Response { id, payload }),
            other => Err(FrameError::UnknownType(other)),
        }
    }
}

#[derive(Default)]
pub(crate) struct Decoder {
    buffer: Vec<u8>,
}

impl Decoder {
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    pub(crate) fn next_frame(&mut self) -> Result<Option<Frame>, FrameError> {
        let Some(header) = self.buffer.first_chunk::<HEADER_SIZE>() else {
            return Ok(None);
        };
        let body_len = u32::from_be_bytes(*header) as usize;
        if body_len > MAX_BODY_SIZE {
            return Err(FrameError::Oversized(body_len));
        }
        let end = HEADER_SIZE + body_len;
        if self.buffer.len() < end {
            return Ok(None);
        }
        let frame = Frame::decode_body(&self.buffer[HEADER_SIZE..end]);
        self.buffer.drain(..end);
        frame.map(Some)
    }
}
```

`client/request.rs`:

```rust
pub(crate) const CLIENT_PROTOCOL_VERSION: u16 = 7;
pub(crate) const VIRTUAL_KEYBOARD_VENDOR_ID: u64 = 0x16c0;
pub(crate) const VIRTUAL_KEYBOARD_PRODUCT_ID: u64 = 0x27db;

pub(crate) type Keys = [u16; 32];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Request {
    KeyboardInitialize { country_code: u64 },
    KeyboardReset,
    Keyboard { modifiers: u8, keys: Keys },
    Consumer(Keys),
    AppleVendorKeyboard(Keys),
    AppleVendorTopCase(Keys),
}

impl Request {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut bytes = CLIENT_PROTOCOL_VERSION.to_le_bytes().to_vec();
        match self {
            Self::KeyboardInitialize { country_code } => {
                bytes.push(0);
                for value in [
                    VIRTUAL_KEYBOARD_VENDOR_ID,
                    VIRTUAL_KEYBOARD_PRODUCT_ID,
                    *country_code,
                ] {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
            Self::KeyboardReset => bytes.push(2),
            Self::Keyboard { modifiers, keys } => {
                bytes.extend_from_slice(&[6, 1, *modifiers, 0]);
                push_keys(&mut bytes, keys);
            }
            Self::Consumer(keys) => {
                bytes.extend_from_slice(&[7, 2]);
                push_keys(&mut bytes, keys);
            }
            Self::AppleVendorKeyboard(keys) => {
                bytes.extend_from_slice(&[8, 4]);
                push_keys(&mut bytes, keys);
            }
            Self::AppleVendorTopCase(keys) => {
                bytes.extend_from_slice(&[9, 3]);
                push_keys(&mut bytes, keys);
            }
        }
        bytes
    }
}

fn push_keys(bytes: &mut Vec<u8>, keys: &Keys) {
    for key in keys {
        bytes.extend_from_slice(&key.to_le_bytes());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    DriverActivated(bool),
    DriverConnected(bool),
    DriverVersionMismatched(bool),
    KeyboardReady(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatusError {
    OddLength(usize),
    UnknownResponse(u8),
}

impl std::fmt::Display for StatusError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OddLength(length) => write!(formatter, "status message has odd length {length}"),
            Self::UnknownResponse(code) => write!(formatter, "unknown status code {code}"),
        }
    }
}

pub(crate) fn parse_status(payload: &[u8]) -> Result<Vec<Status>, StatusError> {
    if payload.len() % 2 != 0 {
        return Err(StatusError::OddLength(payload.len()));
    }
    payload
        .chunks_exact(2)
        .filter_map(|pair| {
            let value = pair[1] != 0;
            match pair[0] {
                0 | 5 => None,
                1 => Some(Ok(Status::DriverActivated(value))),
                2 => Some(Ok(Status::DriverConnected(value))),
                3 => Some(Ok(Status::DriverVersionMismatched(value))),
                4 => Some(Ok(Status::KeyboardReady(value))),
                other => Some(Err(StatusError::UnknownResponse(other))),
            }
        })
        .collect()
}
```

Add `#[allow(dead_code)] mod virtual_hid;` to `platform/macos/mod.rs`. Derived `Debug` does not count as a read for dead-code analysis, which is why the error types implement `Display` by hand.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p qol-keyremap virtual_hid::client`
Expected: PASS, 9 tests.

- [ ] **Step 5: Commit**

```bash
git add plugins/keyremap/src/platform/macos/virtual_hid plugins/keyremap/src/platform/macos/mod.rs
git commit -m "feat(keyremap): encode the pqrs virtual HID client protocol"
```

---

### Task 6: pqrs connection

**Files:**
- Modify: `plugins/keyremap/src/platform/macos/virtual_hid/client/mod.rs`

**Interfaces:**
- Consumes: `Frame`, `Decoder`, `Request`, `Status`, `parse_status` (Task 5).
- Produces:
  - `client::ClientEvent { Status(Status), Disconnected(String) }`
  - `client::Connection::connect(path: &Path, events: Sender<ClientEvent>) -> io::Result<Connection>`
  - `client::Connection::from_stream(stream: UnixStream, events: Sender<ClientEvent>) -> io::Result<Connection>`
  - `client::Connection::send(&self, request: &Request) -> io::Result<()>`
  - Dropping a `Connection` shuts the socket down and stops its threads.

- [ ] **Step 1: Write the failing tests**

Append to `client/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    fn read_frame(stream: &mut UnixStream, decoder: &mut Decoder) -> Frame {
        let mut buffer = [0u8; 256];
        loop {
            if let Some(frame) = decoder.next_frame().unwrap() {
                return frame;
            }
            let read = stream.read(&mut buffer).unwrap();
            assert!(read > 0, "connection closed before a frame arrived");
            decoder.push(&buffer[..read]);
        }
    }

    fn pair() -> (Connection, UnixStream, mpsc::Receiver<ClientEvent>) {
        let (client, mut server) = UnixStream::pair().unwrap();
        server.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let (sender, events) = mpsc::channel();
        let connection = Connection::from_stream(client, sender).unwrap();
        (connection, server, events)
    }

    #[test]
    fn a_pushed_status_request_is_reported_and_acknowledged() {
        let (_connection, mut server, events) = pair();
        server
            .write_all(&Frame::Request { id: 41, payload: vec![4, 1] }.encode())
            .unwrap();

        let event = events.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(event, ClientEvent::Status(Status::KeyboardReady(true))));
        let mut decoder = Decoder::default();
        assert_eq!(
            read_frame(&mut server, &mut decoder),
            Frame::Response { id: 41, payload: Vec::new() }
        );
    }

    #[test]
    fn health_checks_are_answered() {
        let (_connection, mut server, _events) = pair();
        server.write_all(&Frame::HealthCheck.encode()).unwrap();
        let mut decoder = Decoder::default();
        assert_eq!(read_frame(&mut server, &mut decoder), Frame::HealthCheckResponse);
    }

    #[test]
    fn requests_go_out_as_request_frames_with_rising_ids() {
        let (connection, mut server, _events) = pair();
        connection.send(&Request::KeyboardReset).unwrap();
        connection.send(&Request::KeyboardReset).unwrap();
        let mut decoder = Decoder::default();
        let Frame::Request { id: first, payload } = read_frame(&mut server, &mut decoder) else {
            panic!("expected a request frame");
        };
        assert_eq!(payload, vec![7, 0, 2]);
        let Frame::Request { id: second, .. } = read_frame(&mut server, &mut decoder) else {
            panic!("expected a request frame");
        };
        assert!(second > first);
    }

    #[test]
    fn the_daemon_closing_the_socket_is_reported() {
        let (_connection, server, events) = pair();
        drop(server);
        let event = events.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(event, ClientEvent::Disconnected(_)));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p qol-keyremap virtual_hid::client::tests`
Expected: compile error, `cannot find type Connection`.

- [ ] **Step 3: Implement**

`client/mod.rs`, above the tests:

```rust
pub(crate) mod frame;
pub(crate) mod request;

use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use frame::{Decoder, Frame};
use request::{Request, Status};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(3);
const WRITE_TIMEOUT: Duration = Duration::from_millis(50);

#[derive(Debug)]
pub(crate) enum ClientEvent {
    Status(Status),
    Disconnected(String),
}

pub(crate) struct Connection {
    writer: Arc<Mutex<UnixStream>>,
    next_id: AtomicU64,
    closed: Arc<AtomicBool>,
}

impl Connection {
    pub(crate) fn connect(path: &Path, events: Sender<ClientEvent>) -> io::Result<Self> {
        Self::from_stream(UnixStream::connect(path)?, events)
    }

    pub(crate) fn from_stream(stream: UnixStream, events: Sender<ClientEvent>) -> io::Result<Self> {
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        let writer = Arc::new(Mutex::new(stream.try_clone()?));
        let closed = Arc::new(AtomicBool::new(false));
        {
            let writer = Arc::clone(&writer);
            let closed = Arc::clone(&closed);
            std::thread::Builder::new()
                .name("keyremap-pqrs-reader".into())
                .spawn(move || read_loop(stream, &writer, &events, &closed))?;
        }
        {
            let writer = Arc::clone(&writer);
            let closed = Arc::clone(&closed);
            std::thread::Builder::new()
                .name("keyremap-pqrs-heartbeat".into())
                .spawn(move || heartbeat_loop(&writer, &closed))?;
        }
        Ok(Self {
            writer,
            next_id: AtomicU64::new(1),
            closed,
        })
    }

    pub(crate) fn send(&self, request: &Request) -> io::Result<()> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        write_frame(&self.writer, &Frame::Request { id, payload: request.encode() })
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Ok(stream) = self.writer.lock() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

fn write_frame(writer: &Mutex<UnixStream>, frame: &Frame) -> io::Result<()> {
    let mut stream = writer
        .lock()
        .map_err(|_| io::Error::other("pqrs writer lock poisoned"))?;
    stream.write_all(&frame.encode())
}

fn heartbeat_loop(writer: &Mutex<UnixStream>, closed: &AtomicBool) {
    while !closed.load(Ordering::SeqCst) {
        std::thread::sleep(HEARTBEAT_INTERVAL);
        if closed.load(Ordering::SeqCst) || write_frame(writer, &Frame::Heartbeat).is_err() {
            return;
        }
    }
}

fn read_loop(
    mut stream: UnixStream,
    writer: &Mutex<UnixStream>,
    events: &Sender<ClientEvent>,
    closed: &AtomicBool,
) {
    let reason = read_frames(&mut stream, writer, events);
    closed.store(true, Ordering::SeqCst);
    let _ = events.send(ClientEvent::Disconnected(reason));
}

fn read_frames(
    stream: &mut UnixStream,
    writer: &Mutex<UnixStream>,
    events: &Sender<ClientEvent>,
) -> String {
    let mut decoder = Decoder::default();
    let mut buffer = [0u8; 1024];
    loop {
        let read = match stream.read(&mut buffer) {
            Ok(0) => return "the pqrs daemon closed the connection".to_string(),
            Ok(read) => read,
            Err(error) => return format!("reading from the pqrs daemon failed: {error}"),
        };
        decoder.push(&buffer[..read]);
        loop {
            match decoder.next_frame() {
                Ok(None) => break,
                Ok(Some(frame)) => {
                    if let Err(error) = handle_frame(frame, writer, events) {
                        return format!("writing to the pqrs daemon failed: {error}");
                    }
                }
                Err(error) => return format!("the pqrs daemon sent a bad frame: {error}"),
            }
        }
    }
}

fn handle_frame(frame: Frame, writer: &Mutex<UnixStream>, events: &Sender<ClientEvent>) -> io::Result<()> {
    match frame {
        Frame::HealthCheck => write_frame(writer, &Frame::HealthCheckResponse),
        Frame::Request { id, payload } => {
            forward_status(&payload, events);
            write_frame(writer, &Frame::Response { id, payload: Vec::new() })
        }
        Frame::Response { payload, .. } => {
            forward_status(&payload, events);
            Ok(())
        }
        Frame::Heartbeat | Frame::HealthCheckResponse | Frame::UserData(_) => Ok(()),
    }
}

fn forward_status(payload: &[u8], events: &Sender<ClientEvent>) {
    match request::parse_status(payload) {
        Ok(statuses) => {
            for status in statuses {
                let _ = events.send(ClientEvent::Status(status));
            }
        }
        Err(error) => log::warn!("ignoring a pqrs status message: {error}"),
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p qol-keyremap virtual_hid::client`
Expected: PASS, 13 tests.

- [ ] **Step 5: Commit**

```bash
git add plugins/keyremap/src/platform/macos/virtual_hid/client/mod.rs
git commit -m "feat(keyremap): talk to the pqrs virtual HID daemon"
```

---

### Task 7: Helper protocol, watchdog and reports

**Files:**
- Create: `plugins/keyremap/src/platform/macos/hid_helper/mod.rs` (only module declarations in this task)
- Create: `plugins/keyremap/src/platform/macos/hid_helper/protocol.rs`
- Create: `plugins/keyremap/src/platform/macos/hid_helper/watchdog.rs`
- Create: `plugins/keyremap/src/platform/macos/hid_helper/report.rs`
- Modify: `plugins/keyremap/src/platform/macos/mod.rs` (add `#[allow(dead_code)] mod hid_helper;`)

**Interfaces:**
- Consumes: `request::{Keys, Request}` (Task 5).
- Produces:
  - `protocol::{PROTOCOL_VERSION: u32 = 1, SOCKET_PATH, PAGE_KEYBOARD = 0x07, PAGE_CONSUMER = 0x0C, PAGE_APPLE_VENDOR_KEYBOARD = 0xFF01, PAGE_APPLE_VENDOR_TOP_CASE = 0x00FF}`
  - `protocol::Role { Session, Status }`
  - `protocol::ToHelper { Hello { protocol: u32, role: Role }, Heartbeat, Emit { usage_page: u16, usage: u16, pressed: bool }, CapsLockLight { on: bool } }`
  - `protocol::ToDaemon { Welcome { protocol: u32 }, Refused { protocol: u32, reason: String }, Key { usage_page: u16, usage: u16, pressed: bool, apple: bool }, Seized { active: bool }, Status(HelperStatus) }`
  - `protocol::HelperStatus { protocol: u32, virtual_keyboard_ready: bool, input_monitoring: bool, seized: Vec<String>, conflicts: Vec<String> }`
  - `protocol::write_message<T: Serialize>(&mut impl Write, &T) -> io::Result<()>`, `protocol::parse_message<T: DeserializeOwned>(&str) -> serde_json::Result<T>`
  - `watchdog::{Watchdog, Action { Seize, Release, Hold }, SILENCE_LIMIT}` with `session_opened(u64)`, `session_closed(u64)`, `heartbeat(u64, Instant)`, `set_keyboard_ready(bool)`, `set_input_monitoring(bool)`, `device_arrived()`, `tick(Instant) -> Action`, `is_current(u64) -> bool`
  - `report::ReportState::default()`, `apply(&mut self, page: u16, usage: u16, pressed: bool) -> Option<Request>`, `clear(&mut self) -> Vec<Request>`
  - `report::PressedKeys::default()`, `record(&mut self, page: u16, usage: u16, pressed: bool)`, `drain(&mut self) -> Vec<(u16, u16)>`

- [ ] **Step 1: Write the failing tests**

`protocol.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_messages_have_a_stable_wire_form() {
        let key = ToDaemon::Key { usage_page: 7, usage: 4, pressed: true, apple: true };
        let mut line = Vec::new();
        write_message(&mut line, &key).unwrap();
        assert_eq!(
            String::from_utf8(line).unwrap(),
            "{\"type\":\"key\",\"usage_page\":7,\"usage\":4,\"pressed\":true,\"apple\":true}\n"
        );
    }

    #[test]
    fn every_message_round_trips() {
        let to_helper = [
            ToHelper::Hello { protocol: PROTOCOL_VERSION, role: Role::Session },
            ToHelper::Hello { protocol: PROTOCOL_VERSION, role: Role::Status },
            ToHelper::Heartbeat,
            ToHelper::Emit { usage_page: 0x0C, usage: 0xCD, pressed: false },
            ToHelper::CapsLockLight { on: true },
        ];
        for message in to_helper {
            let mut line = Vec::new();
            write_message(&mut line, &message).unwrap();
            let parsed: ToHelper = parse_message(std::str::from_utf8(&line).unwrap()).unwrap();
            assert_eq!(parsed, message);
        }
        let to_daemon = [
            ToDaemon::Welcome { protocol: 1 },
            ToDaemon::Refused { protocol: 2, reason: "old daemon".to_string() },
            ToDaemon::Seized { active: true },
            ToDaemon::Status(HelperStatus {
                protocol: 1,
                virtual_keyboard_ready: true,
                input_monitoring: false,
                seized: vec!["Apple Internal Keyboard / Trackpad".to_string()],
                conflicts: Vec::new(),
            }),
        ];
        for message in to_daemon {
            let mut line = Vec::new();
            write_message(&mut line, &message).unwrap();
            let parsed: ToDaemon = parse_message(std::str::from_utf8(&line).unwrap()).unwrap();
            assert_eq!(parsed, message);
        }
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(parse_message::<ToHelper>("{\"type\":\"nope\"}").is_err());
        assert!(parse_message::<ToHelper>("").is_err());
    }
}
```

`watchdog.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn ready() -> Watchdog {
        let mut watchdog = Watchdog::default();
        watchdog.set_keyboard_ready(true);
        watchdog.set_input_monitoring(true);
        watchdog
    }

    #[test]
    fn seizes_only_after_a_heartbeat_with_the_keyboard_ready() {
        let start = Instant::now();
        let mut watchdog = ready();
        watchdog.session_opened(1);
        assert_eq!(watchdog.tick(start), Action::Hold);
        watchdog.heartbeat(1, start);
        assert_eq!(watchdog.tick(start), Action::Seize);
        assert_eq!(watchdog.tick(start + Duration::from_millis(25)), Action::Hold);
    }

    #[test]
    fn releases_after_200ms_of_silence_and_reseizes_on_the_next_heartbeat() {
        let start = Instant::now();
        let mut watchdog = ready();
        watchdog.session_opened(1);
        watchdog.heartbeat(1, start);
        assert_eq!(watchdog.tick(start), Action::Seize);
        assert_eq!(watchdog.tick(start + SILENCE_LIMIT), Action::Hold);
        assert_eq!(
            watchdog.tick(start + SILENCE_LIMIT + Duration::from_millis(1)),
            Action::Release
        );
        let later = start + Duration::from_secs(1);
        watchdog.heartbeat(1, later);
        assert_eq!(watchdog.tick(later), Action::Seize);
    }

    #[test]
    fn releases_at_once_when_the_virtual_keyboard_goes_away() {
        let start = Instant::now();
        let mut watchdog = ready();
        watchdog.session_opened(1);
        watchdog.heartbeat(1, start);
        assert_eq!(watchdog.tick(start), Action::Seize);
        watchdog.set_keyboard_ready(false);
        assert_eq!(watchdog.tick(start), Action::Release);
    }

    #[test]
    fn releases_when_the_session_closes() {
        let start = Instant::now();
        let mut watchdog = ready();
        watchdog.session_opened(1);
        watchdog.heartbeat(1, start);
        assert_eq!(watchdog.tick(start), Action::Seize);
        watchdog.session_closed(1);
        assert_eq!(watchdog.tick(start), Action::Release);
    }

    #[test]
    fn no_input_monitoring_never_seizes() {
        let start = Instant::now();
        let mut watchdog = ready();
        watchdog.set_input_monitoring(false);
        watchdog.session_opened(1);
        watchdog.heartbeat(1, start);
        assert_eq!(watchdog.tick(start), Action::Hold);
    }

    #[test]
    fn a_stale_session_cannot_keep_or_drop_the_seize() {
        let start = Instant::now();
        let mut watchdog = ready();
        watchdog.session_opened(1);
        watchdog.heartbeat(1, start);
        assert_eq!(watchdog.tick(start), Action::Seize);

        watchdog.session_opened(2);
        watchdog.heartbeat(1, start);
        assert!(!watchdog.is_current(1));
        assert_eq!(watchdog.tick(start), Action::Release);

        watchdog.heartbeat(2, start);
        assert_eq!(watchdog.tick(start), Action::Seize);
        watchdog.session_closed(1);
        assert_eq!(watchdog.tick(start), Action::Hold);
    }

    #[test]
    fn device_arrival_while_seized_asks_for_a_seize() {
        let start = Instant::now();
        let mut watchdog = ready();
        watchdog.session_opened(1);
        watchdog.heartbeat(1, start);
        assert_eq!(watchdog.tick(start), Action::Seize);
        watchdog.device_arrived();
        assert_eq!(watchdog.tick(start), Action::Seize);
        assert_eq!(watchdog.tick(start), Action::Hold);
    }

    #[test]
    fn device_arrival_while_released_does_nothing() {
        let mut watchdog = ready();
        watchdog.device_arrived();
        assert_eq!(watchdog.tick(Instant::now()), Action::Hold);
    }
}
```

`report.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::macos::hid_helper::protocol::{
        PAGE_APPLE_VENDOR_TOP_CASE, PAGE_CONSUMER, PAGE_KEYBOARD,
    };

    fn keys(first: &[u16]) -> Keys {
        let mut keys = [0u16; 32];
        keys[..first.len()].copy_from_slice(first);
        keys
    }

    #[test]
    fn modifiers_go_into_the_modifier_byte() {
        let mut state = ReportState::default();
        assert_eq!(
            state.apply(PAGE_KEYBOARD, 0xE3, true),
            Some(Request::Keyboard { modifiers: 0x08, keys: keys(&[]) })
        );
        assert_eq!(
            state.apply(PAGE_KEYBOARD, 0x06, true),
            Some(Request::Keyboard { modifiers: 0x08, keys: keys(&[0x06]) })
        );
        assert_eq!(
            state.apply(PAGE_KEYBOARD, 0xE3, false),
            Some(Request::Keyboard { modifiers: 0, keys: keys(&[0x06]) })
        );
    }

    #[test]
    fn a_released_key_frees_its_slot_and_duplicates_are_ignored() {
        let mut state = ReportState::default();
        state.apply(PAGE_KEYBOARD, 0x04, true);
        state.apply(PAGE_KEYBOARD, 0x04, true);
        state.apply(PAGE_KEYBOARD, 0x05, true);
        assert_eq!(
            state.apply(PAGE_KEYBOARD, 0x04, false),
            Some(Request::Keyboard { modifiers: 0, keys: keys(&[0, 0x05]) })
        );
    }

    #[test]
    fn other_pages_use_their_own_reports() {
        let mut state = ReportState::default();
        assert_eq!(
            state.apply(PAGE_CONSUMER, 0xCD, true),
            Some(Request::Consumer(keys(&[0xCD])))
        );
        assert_eq!(
            state.apply(PAGE_APPLE_VENDOR_TOP_CASE, 0x03, true),
            Some(Request::AppleVendorTopCase(keys(&[0x03])))
        );
        assert_eq!(state.apply(0x0001, 0x80, true), None);
    }

    #[test]
    fn clear_releases_every_page_that_had_keys() {
        let mut state = ReportState::default();
        state.apply(PAGE_KEYBOARD, 0xE3, true);
        state.apply(PAGE_CONSUMER, 0xE9, true);
        assert_eq!(
            state.clear(),
            vec![
                Request::Keyboard { modifiers: 0, keys: keys(&[]) },
                Request::Consumer(keys(&[])),
            ]
        );
        assert!(state.clear().is_empty());
    }

    #[test]
    fn unplugging_releases_what_the_device_held() {
        let mut pressed = PressedKeys::default();
        pressed.record(PAGE_KEYBOARD, 0x04, true);
        pressed.record(PAGE_KEYBOARD, 0xE0, true);
        pressed.record(PAGE_KEYBOARD, 0x04, false);
        pressed.record(PAGE_CONSUMER, 0xE9, true);
        assert_eq!(pressed.drain(), vec![(PAGE_KEYBOARD, 0xE0), (PAGE_CONSUMER, 0xE9)]);
        assert!(pressed.drain().is_empty());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p qol-keyremap hid_helper`
Expected: compile error, `cannot find type ToDaemon`.

- [ ] **Step 3: Implement**

`hid_helper/mod.rs` in this task:

```rust
pub(crate) mod protocol;
pub(crate) mod report;
pub(crate) mod watchdog;
```

`protocol.rs`:

```rust
use std::io::{self, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub(crate) const PROTOCOL_VERSION: u32 = 1;
pub(crate) const SOCKET_PATH: &str = "/var/run/com.qol-tools.keyremap.hid-helper.sock";

pub(crate) const PAGE_KEYBOARD: u16 = 0x07;
pub(crate) const PAGE_CONSUMER: u16 = 0x0C;
pub(crate) const PAGE_APPLE_VENDOR_KEYBOARD: u16 = 0xFF01;
pub(crate) const PAGE_APPLE_VENDOR_TOP_CASE: u16 = 0x00FF;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Role {
    Session,
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ToHelper {
    Hello { protocol: u32, role: Role },
    Heartbeat,
    Emit { usage_page: u16, usage: u16, pressed: bool },
    CapsLockLight { on: bool },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum ToDaemon {
    Welcome { protocol: u32 },
    Refused { protocol: u32, reason: String },
    Key { usage_page: u16, usage: u16, pressed: bool, apple: bool },
    Seized { active: bool },
    Status(HelperStatus),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct HelperStatus {
    pub(crate) protocol: u32,
    pub(crate) virtual_keyboard_ready: bool,
    pub(crate) input_monitoring: bool,
    pub(crate) seized: Vec<String>,
    pub(crate) conflicts: Vec<String>,
}

pub(crate) fn write_message<T: Serialize>(writer: &mut impl Write, message: &T) -> io::Result<()> {
    let mut line = serde_json::to_vec(message).map_err(io::Error::other)?;
    line.push(b'\n');
    writer.write_all(&line)
}

pub(crate) fn parse_message<T: DeserializeOwned>(line: &str) -> serde_json::Result<T> {
    serde_json::from_str(line.trim_end())
}
```

`watchdog.rs`:

```rust
use std::time::{Duration, Instant};

pub(crate) const SILENCE_LIMIT: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Seize,
    Release,
    Hold,
}

#[derive(Debug, Default)]
pub(crate) struct Watchdog {
    session: Option<u64>,
    last_heartbeat: Option<Instant>,
    keyboard_ready: bool,
    input_monitoring: bool,
    seized: bool,
    devices_changed: bool,
}

impl Watchdog {
    pub(crate) fn session_opened(&mut self, generation: u64) {
        self.session = Some(generation);
        self.last_heartbeat = None;
    }

    pub(crate) fn session_closed(&mut self, generation: u64) {
        if self.is_current(generation) {
            self.session = None;
            self.last_heartbeat = None;
        }
    }

    pub(crate) fn heartbeat(&mut self, generation: u64, now: Instant) {
        if self.is_current(generation) {
            self.last_heartbeat = Some(now);
        }
    }

    pub(crate) fn is_current(&self, generation: u64) -> bool {
        self.session == Some(generation)
    }

    pub(crate) fn set_keyboard_ready(&mut self, ready: bool) {
        self.keyboard_ready = ready;
    }

    pub(crate) fn set_input_monitoring(&mut self, granted: bool) {
        self.input_monitoring = granted;
    }

    pub(crate) fn device_arrived(&mut self) {
        self.devices_changed = true;
    }

    pub(crate) fn tick(&mut self, now: Instant) -> Action {
        let heard_recently = self
            .last_heartbeat
            .is_some_and(|at| now.saturating_duration_since(at) <= SILENCE_LIMIT);
        let wanted = self.keyboard_ready && self.input_monitoring && heard_recently;
        let action = match (self.seized, wanted) {
            (false, true) => Action::Seize,
            (true, false) => Action::Release,
            (true, true) if self.devices_changed => Action::Seize,
            _ => Action::Hold,
        };
        self.devices_changed = false;
        self.seized = wanted;
        action
    }
}
```

`last_heartbeat` is only ever set for the current session, so `heard_recently` already implies a live session.

`report.rs`:

```rust
use crate::platform::macos::hid_helper::protocol::{
    PAGE_APPLE_VENDOR_KEYBOARD, PAGE_APPLE_VENDOR_TOP_CASE, PAGE_CONSUMER, PAGE_KEYBOARD,
};
use crate::platform::macos::virtual_hid::client::request::{Keys, Request};

const FIRST_MODIFIER: u16 = 0xE0;
const LAST_MODIFIER: u16 = 0xE7;

#[derive(Default)]
struct KeySet(Keys);

impl KeySet {
    fn set(&mut self, usage: u16, pressed: bool) {
        if pressed {
            if !self.0.contains(&usage) {
                if let Some(slot) = self.0.iter_mut().find(|slot| **slot == 0) {
                    *slot = usage;
                }
            }
        } else {
            for slot in self.0.iter_mut().filter(|slot| **slot == usage) {
                *slot = 0;
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.0.iter().all(|slot| *slot == 0)
    }
}

#[derive(Default)]
pub(crate) struct ReportState {
    modifiers: u8,
    keyboard: KeySet,
    consumer: KeySet,
    apple_keyboard: KeySet,
    top_case: KeySet,
}

impl ReportState {
    pub(crate) fn apply(&mut self, page: u16, usage: u16, pressed: bool) -> Option<Request> {
        match page {
            PAGE_KEYBOARD if (FIRST_MODIFIER..=LAST_MODIFIER).contains(&usage) => {
                let bit = 1u8 << (usage - FIRST_MODIFIER);
                if pressed {
                    self.modifiers |= bit;
                } else {
                    self.modifiers &= !bit;
                }
                Some(self.keyboard_report())
            }
            PAGE_KEYBOARD => {
                self.keyboard.set(usage, pressed);
                Some(self.keyboard_report())
            }
            PAGE_CONSUMER => {
                self.consumer.set(usage, pressed);
                Some(Request::Consumer(self.consumer.0))
            }
            PAGE_APPLE_VENDOR_KEYBOARD => {
                self.apple_keyboard.set(usage, pressed);
                Some(Request::AppleVendorKeyboard(self.apple_keyboard.0))
            }
            PAGE_APPLE_VENDOR_TOP_CASE => {
                self.top_case.set(usage, pressed);
                Some(Request::AppleVendorTopCase(self.top_case.0))
            }
            _ => None,
        }
    }

    pub(crate) fn clear(&mut self) -> Vec<Request> {
        let mut releases = Vec::new();
        if self.modifiers != 0 || !self.keyboard.is_empty() {
            releases.push(Request::Keyboard { modifiers: 0, keys: [0; 32] });
        }
        if !self.consumer.is_empty() {
            releases.push(Request::Consumer([0; 32]));
        }
        if !self.apple_keyboard.is_empty() {
            releases.push(Request::AppleVendorKeyboard([0; 32]));
        }
        if !self.top_case.is_empty() {
            releases.push(Request::AppleVendorTopCase([0; 32]));
        }
        *self = Self::default();
        releases
    }

    fn keyboard_report(&self) -> Request {
        Request::Keyboard { modifiers: self.modifiers, keys: self.keyboard.0 }
    }
}

#[derive(Default)]
pub(crate) struct PressedKeys {
    keys: Vec<(u16, u16)>,
}

impl PressedKeys {
    pub(crate) fn record(&mut self, page: u16, usage: u16, pressed: bool) {
        self.keys.retain(|key| *key != (page, usage));
        if pressed {
            self.keys.push((page, usage));
        }
    }

    pub(crate) fn drain(&mut self) -> Vec<(u16, u16)> {
        std::mem::take(&mut self.keys)
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p qol-keyremap hid_helper`
Expected: PASS, 16 tests.

- [ ] **Step 5: Commit**

```bash
git add plugins/keyremap/src/platform/macos/hid_helper plugins/keyremap/src/platform/macos/mod.rs
git commit -m "feat(keyremap): define the keyboard helper protocol and watchdog"
```

---

### Task 8: Helper runtime and the `hid-helper` command

**Files:**
- Modify: `plugins/keyremap/src/platform/macos/hid_helper/mod.rs`
- Create: `plugins/keyremap/src/platform/macos/hid_helper/console.rs`
- Create: `plugins/keyremap/src/platform/macos/hid_helper/devices.rs`
- Create: `plugins/keyremap/src/platform/macos/hid_helper/server.rs`
- Modify: `plugins/keyremap/src/platform/mod.rs` (trait), `plugins/keyremap/src/platform/{macos,linux,windows,fallback}/mod.rs`
- Modify: `plugins/keyremap/src/cli.rs`

**Interfaces:**
- Consumes: `protocol`, `watchdog`, `report` (Task 7), `client::{Connection, ClientEvent}`, `request::{Request, Status}` (Tasks 5, 6), `virtual_hid::PQRS_SOCKET`.
- Produces:
  - `hid_helper::run() -> anyhow::Result<()>`: returns only on error; otherwise runs the main run loop forever.
  - `hid_helper::is_root() -> bool`
  - `PlatformAdapter::hid_helper(&self) -> Result<CommandResult>`
  - CLI command `hid-helper`.

Threads: the main thread owns every IOKit object and runs a 25 ms run-loop timer that applies the watchdog. The server thread accepts connections; each session gets a reader thread. The pqrs supervisor thread keeps the virtual keyboard alive. All shared state sits behind one mutex in `Shared`; every socket write under it has a 50 ms timeout so a stalled daemon cannot block the watchdog.

- [ ] **Step 1: Write the failing tests**

`console.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_login_window_and_root_are_not_console_users() {
        assert_eq!(console_owner("loginwindow", 501, 20), None);
        assert_eq!(console_owner("root", 0, 0), None);
        assert_eq!(console_owner("kaho", 501, 20), Some(ConsoleUser { uid: 501, gid: 20 }));
    }
}
```

`devices.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_key_pages_are_forwarded() {
        assert!(forwarded(PAGE_KEYBOARD, 0x04));
        assert!(forwarded(PAGE_KEYBOARD, 0xE7));
        assert!(!forwarded(PAGE_KEYBOARD, 0x01));
        assert!(!forwarded(PAGE_KEYBOARD, 0xFFFF));
        assert!(forwarded(PAGE_CONSUMER, 0xCD));
        assert!(forwarded(PAGE_APPLE_VENDOR_TOP_CASE, 0x03));
        assert!(forwarded(PAGE_APPLE_VENDOR_KEYBOARD, 0x01));
        assert!(!forwarded(PAGE_CONSUMER, 0));
        assert!(!forwarded(0x01, 0x30));
    }

    #[test]
    fn the_virtual_keyboard_itself_is_never_seized() {
        assert!(is_virtual_keyboard(0x16c0, 0x27db, "anything"));
        assert!(is_virtual_keyboard(0x05ac, 0x0341, "Karabiner DriverKit VirtualHIDKeyboard"));
        assert!(!is_virtual_keyboard(0x05ac, 0x0341, "Apple Internal Keyboard / Trackpad"));
    }
}
```

`server.rs` tests, over a socket pair so no root path is touched:

```rust
#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;

    use super::*;

    extern "C" {
        fn getuid() -> u32;
        fn getgid() -> u32;
    }

    fn me() -> ConsoleUser {
        ConsoleUser { uid: unsafe { getuid() }, gid: unsafe { getgid() } }
    }

    fn say(client: &mut UnixStream, message: &ToHelper) {
        protocol::write_message(client, message).unwrap();
    }

    fn hear(client: &UnixStream) -> ToDaemon {
        let mut line = String::new();
        BufReader::new(client).read_line(&mut line).unwrap();
        protocol::parse_message(&line).unwrap()
    }

    #[test]
    fn status_connections_get_the_status() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let shared = Arc::new(Shared::default());
        say(&mut client, &ToHelper::Hello { protocol: PROTOCOL_VERSION, role: Role::Status });
        accept(server, me(), &shared, &mut 0).unwrap();
        let ToDaemon::Status(status) = hear(&client) else { panic!("expected status") };
        assert_eq!(status.protocol, PROTOCOL_VERSION);
        assert!(!status.virtual_keyboard_ready);
    }

    #[test]
    fn another_protocol_is_refused_with_the_helper_version() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let shared = Arc::new(Shared::default());
        say(&mut client, &ToHelper::Hello { protocol: PROTOCOL_VERSION + 1, role: Role::Session });
        accept(server, me(), &shared, &mut 0).unwrap();
        assert!(matches!(
            hear(&client),
            ToDaemon::Refused { protocol, .. } if protocol == PROTOCOL_VERSION
        ));
    }

    #[test]
    fn a_session_is_welcomed_and_its_heartbeats_reach_the_watchdog() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let shared = Arc::new(Shared::default());
        let mut generation = 0;
        say(&mut client, &ToHelper::Hello { protocol: PROTOCOL_VERSION, role: Role::Session });
        accept(server, me(), &shared, &mut generation).unwrap();
        assert!(matches!(hear(&client), ToDaemon::Welcome { .. }));
        assert_eq!(generation, 1);
        assert!(shared.lock().watchdog.is_current(1));

        drop(client);
        for _ in 0..50 {
            if !shared.lock().watchdog.is_current(1) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("closing the session did not reach the watchdog");
    }

    #[test]
    fn a_peer_from_another_user_is_refused() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let shared = Arc::new(Shared::default());
        say(&mut client, &ToHelper::Hello { protocol: PROTOCOL_VERSION, role: Role::Status });
        let stranger = ConsoleUser { uid: me().uid + 1, gid: me().gid };
        assert!(accept(server, stranger, &shared, &mut 0).is_err());
        let _ = client.flush();
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p qol-keyremap hid_helper`
Expected: compile error, `cannot find function accept`.

- [ ] **Step 3: Implement the shared state and entry point**

`hid_helper/mod.rs`:

```rust
mod console;
mod devices;
pub(crate) mod protocol;
pub(crate) mod report;
mod server;
pub(crate) mod watchdog;

use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use crate::platform::macos::virtual_hid::client::request::{Request, Status};
use crate::platform::macos::virtual_hid::client::{ClientEvent, Connection};
use crate::platform::macos::virtual_hid::PQRS_SOCKET;
use protocol::{HelperStatus, ToDaemon, PAGE_APPLE_VENDOR_TOP_CASE, PAGE_KEYBOARD, PROTOCOL_VERSION};
use report::ReportState;
use watchdog::{Action, Watchdog};

const RECONNECT_INTERVAL: Duration = Duration::from_secs(1);
const COUNTRY_CODE_WAIT: Duration = Duration::from_secs(2);

extern "C" {
    fn geteuid() -> u32;
}

pub(crate) fn is_root() -> bool {
    unsafe { geteuid() == 0 }
}

pub(crate) fn run() -> Result<()> {
    if !is_root() {
        bail!("hid-helper must run as root; install it with `sudo qol-keyremap install-hid-helper`");
    }
    devices::request_input_monitoring();
    let shared = Arc::new(Shared::default());
    let server = Arc::clone(&shared);
    std::thread::Builder::new()
        .name("keyremap-helper-server".into())
        .spawn(move || server::serve(&server))?;
    let supervisor = Arc::clone(&shared);
    std::thread::Builder::new()
        .name("keyremap-helper-vhid".into())
        .spawn(move || supervise_virtual_keyboard(&supervisor))?;
    log::info!("keyboard helper started, protocol {PROTOCOL_VERSION}");
    devices::run(shared)
}

#[derive(Default)]
pub(super) struct Shared {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    watchdog: Watchdog,
    reports: ReportState,
    vhid: Option<Connection>,
    session: Option<(u64, UnixStream)>,
    caps_light: Option<bool>,
    keyboard_ready: bool,
    input_monitoring: bool,
    country_code: Option<u64>,
    seized: Vec<String>,
    conflicts: Vec<String>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn open_session(&self, generation: u64, writer: UnixStream) {
        let mut state = self.lock();
        if let Some((_, previous)) = state.session.take() {
            let _ = previous.shutdown(Shutdown::Both);
        }
        state.watchdog.session_opened(generation);
        state.session = Some((generation, writer));
    }

    fn close_session(&self, generation: u64) {
        let mut state = self.lock();
        state.watchdog.session_closed(generation);
        if state.session.as_ref().is_some_and(|(current, _)| *current == generation) {
            state.session = None;
        }
    }

    fn drop_session(&self) {
        let mut state = self.lock();
        if let Some((generation, stream)) = state.session.take() {
            let _ = stream.shutdown(Shutdown::Both);
            state.watchdog.session_closed(generation);
        }
    }

    fn heartbeat(&self, generation: u64) {
        self.lock().watchdog.heartbeat(generation, Instant::now());
    }

    fn emit(&self, generation: u64, page: u16, usage: u16, pressed: bool) {
        let mut state = self.lock();
        if state.watchdog.is_current(generation) {
            state.post(page, usage, pressed);
        }
    }

    fn set_caps_light(&self, generation: u64, on: bool) {
        let mut state = self.lock();
        if state.watchdog.is_current(generation) {
            state.caps_light = Some(on);
        }
    }

    fn set_input_monitoring(&self, granted: bool) {
        let mut state = self.lock();
        state.input_monitoring = granted;
        state.watchdog.set_input_monitoring(granted);
    }

    fn status(&self) -> HelperStatus {
        let state = self.lock();
        HelperStatus {
            protocol: PROTOCOL_VERSION,
            virtual_keyboard_ready: state.keyboard_ready,
            input_monitoring: state.input_monitoring,
            seized: state.seized.clone(),
            conflicts: state.conflicts.clone(),
        }
    }

    fn tick(&self) -> (Action, Option<bool>) {
        let mut state = self.lock();
        let action = state.watchdog.tick(Instant::now());
        (action, state.caps_light.take())
    }

    fn device_arrived(&self, country_code: u64) {
        let mut state = self.lock();
        state.watchdog.device_arrived();
        state.country_code.get_or_insert(country_code);
    }

    fn publish_devices(&self, seized: Vec<String>, conflicts: Vec<String>) {
        let mut state = self.lock();
        state.seized = seized;
        state.conflicts = conflicts;
    }

    fn announce_seized(&self, active: bool) {
        let mut state = self.lock();
        if !active {
            for release in state.reports.clear() {
                state.send_vhid(&release);
            }
            state.send_vhid(&Request::KeyboardReset);
        }
        state.tell_session(&ToDaemon::Seized { active });
    }

    fn forward(&self, page: u16, usage: u16, pressed: bool, apple: bool) {
        let mut state = self.lock();
        match page {
            PAGE_KEYBOARD | PAGE_APPLE_VENDOR_TOP_CASE => state.tell_session(&ToDaemon::Key {
                usage_page: page,
                usage,
                pressed,
                apple,
            }),
            _ => state.post(page, usage, pressed),
        }
    }

    fn set_vhid(&self, connection: Option<Connection>) {
        let mut state = self.lock();
        if connection.is_none() {
            state.keyboard_ready = false;
            state.watchdog.set_keyboard_ready(false);
        }
        state.vhid = connection;
    }

    fn set_keyboard_ready(&self, ready: bool) {
        let mut state = self.lock();
        state.keyboard_ready = ready;
        state.watchdog.set_keyboard_ready(ready);
    }

    fn country_code(&self) -> Option<u64> {
        self.lock().country_code
    }
}

impl State {
    fn post(&mut self, page: u16, usage: u16, pressed: bool) {
        if let Some(report) = self.reports.apply(page, usage, pressed) {
            self.send_vhid(&report);
        }
    }

    fn send_vhid(&self, request: &Request) {
        if let Some(vhid) = &self.vhid {
            if let Err(error) = vhid.send(request) {
                log::warn!("posting to the virtual keyboard failed: {error}");
            }
        }
    }

    fn tell_session(&mut self, message: &ToDaemon) {
        let Some((generation, stream)) = self.session.as_mut() else {
            return;
        };
        let generation = *generation;
        if let Err(error) = protocol::write_message(stream, message) {
            log::warn!("dropping keyremap session {generation}: {error}");
            if let Some((_, stream)) = self.session.take() {
                let _ = stream.shutdown(Shutdown::Both);
            }
            self.watchdog.session_closed(generation);
        }
    }
}

fn supervise_virtual_keyboard(shared: &Shared) {
    let mut last_error = String::new();
    loop {
        let (sender, events) = mpsc::channel();
        match Connection::connect(Path::new(PQRS_SOCKET), sender) {
            Ok(connection) => {
                last_error.clear();
                let country_code = wait_for_country_code(shared);
                match connection.send(&Request::KeyboardInitialize { country_code }) {
                    Ok(()) => {
                        log::info!("creating the virtual keyboard, country code {country_code}");
                        shared.set_vhid(Some(connection));
                        watch_virtual_keyboard(shared, &events);
                        shared.set_vhid(None);
                    }
                    Err(error) => log::warn!("creating the virtual keyboard failed: {error}"),
                }
            }
            Err(error) => {
                let message = error.to_string();
                if message != last_error {
                    log::warn!("the pqrs daemon is not reachable: {message}");
                    last_error = message;
                }
            }
        }
        std::thread::sleep(RECONNECT_INTERVAL);
    }
}

fn wait_for_country_code(shared: &Shared) -> u64 {
    let deadline = Instant::now() + COUNTRY_CODE_WAIT;
    loop {
        if let Some(country_code) = shared.country_code() {
            return country_code;
        }
        if Instant::now() >= deadline {
            return 0;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn watch_virtual_keyboard(shared: &Shared, events: &Receiver<ClientEvent>) {
    for event in events.iter() {
        match event {
            ClientEvent::Status(Status::KeyboardReady(ready)) => {
                log::info!("virtual keyboard ready: {ready}");
                shared.set_keyboard_ready(ready);
            }
            ClientEvent::Status(Status::DriverActivated(active)) => {
                log::info!("virtual HID driver activated: {active}");
            }
            ClientEvent::Status(Status::DriverConnected(connected)) => {
                log::info!("virtual HID driver connected: {connected}");
            }
            ClientEvent::Status(Status::DriverVersionMismatched(mismatched)) => {
                if mismatched {
                    log::error!("the virtual HID driver does not match the pqrs daemon; reinstall the driver package");
                }
            }
            ClientEvent::Disconnected(reason) => {
                log::warn!("lost the pqrs daemon: {reason}");
                return;
            }
        }
    }
}
```

- [ ] **Step 4: Implement the console user and the socket server**

`console.rs`:

```rust
use std::ffi::c_void;

use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ConsoleUser {
    pub(super) uid: u32,
    pub(super) gid: u32,
}

#[link(name = "SystemConfiguration", kind = "framework")]
extern "C" {
    fn SCDynamicStoreCopyConsoleUser(store: *const c_void, uid: *mut u32, gid: *mut u32) -> CFStringRef;
}

pub(super) fn console_user() -> Option<ConsoleUser> {
    let (mut uid, mut gid) = (0, 0);
    let name = unsafe { SCDynamicStoreCopyConsoleUser(std::ptr::null(), &mut uid, &mut gid) };
    if name.is_null() {
        return None;
    }
    let name = unsafe { CFString::wrap_under_create_rule(name) }.to_string();
    console_owner(&name, uid, gid)
}

fn console_owner(name: &str, uid: u32, gid: u32) -> Option<ConsoleUser> {
    (name != "loginwindow" && uid != 0).then_some(ConsoleUser { uid, gid })
}
```

`server.rs`:

```rust
use std::io::{self, BufRead, BufReader};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, ensure, Context, Result};

use super::console::{self, ConsoleUser};
use super::devices;
use super::protocol::{self, Role, ToDaemon, ToHelper, PROTOCOL_VERSION, SOCKET_PATH};
use super::Shared;

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const HOUSEKEEPING_INTERVAL: Duration = Duration::from_secs(1);
const HELLO_TIMEOUT: Duration = Duration::from_secs(1);
const WRITE_TIMEOUT: Duration = Duration::from_millis(50);

extern "C" {
    fn getpeereid(socket: i32, uid: *mut u32, gid: *mut u32) -> i32;
}

pub(super) fn serve(shared: &Arc<Shared>) {
    let mut generation = 0;
    loop {
        shared.set_input_monitoring(devices::input_monitoring_granted());
        let Some(owner) = console::console_user() else {
            std::thread::sleep(HOUSEKEEPING_INTERVAL);
            continue;
        };
        match bind(owner) {
            Ok(listener) => {
                log::info!("listening on {SOCKET_PATH} for uid {}", owner.uid);
                accept_until_owner_changes(&listener, owner, shared, &mut generation);
            }
            Err(error) => {
                log::error!("cannot listen on {SOCKET_PATH}: {error}");
                std::thread::sleep(HOUSEKEEPING_INTERVAL);
            }
        }
    }
}

fn bind(owner: ConsoleUser) -> io::Result<UnixListener> {
    match std::fs::remove_file(SOCKET_PATH) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let listener = UnixListener::bind(SOCKET_PATH)?;
    std::os::unix::fs::chown(SOCKET_PATH, Some(owner.uid), Some(owner.gid))?;
    std::fs::set_permissions(SOCKET_PATH, std::fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn accept_until_owner_changes(
    listener: &UnixListener,
    owner: ConsoleUser,
    shared: &Arc<Shared>,
    generation: &mut u64,
) {
    let mut next_housekeeping = Instant::now() + HOUSEKEEPING_INTERVAL;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Err(error) = accept(stream, owner, shared, generation) {
                    log::warn!("refused a keyremap connection: {error:#}");
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(POLL_INTERVAL),
            Err(error) => {
                log::warn!("accept on {SOCKET_PATH} failed: {error}");
                std::thread::sleep(POLL_INTERVAL);
            }
        }
        if Instant::now() >= next_housekeeping {
            next_housekeeping = Instant::now() + HOUSEKEEPING_INTERVAL;
            shared.set_input_monitoring(devices::input_monitoring_granted());
            if console::console_user() != Some(owner) {
                log::info!("the console user changed; re-creating {SOCKET_PATH}");
                shared.drop_session();
                return;
            }
        }
    }
}

fn accept(stream: UnixStream, owner: ConsoleUser, shared: &Arc<Shared>, generation: &mut u64) -> Result<()> {
    stream.set_nonblocking(false)?;
    let peer = peer_uid(&stream)?;
    ensure!(peer == owner.uid, "peer uid {peer} is not the console user {}", owner.uid);
    stream.set_read_timeout(Some(HELLO_TIMEOUT))?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    let mut writer = stream.try_clone()?;
    let mut lines = BufReader::new(stream).lines();
    let hello = lines.next().context("the client closed before saying hello")??;
    let ToHelper::Hello { protocol, role } = protocol::parse_message(&hello)? else {
        bail!("the first message was not hello");
    };
    if protocol != PROTOCOL_VERSION {
        protocol::write_message(
            &mut writer,
            &ToDaemon::Refused {
                protocol: PROTOCOL_VERSION,
                reason: format!("the keyboard helper speaks protocol {PROTOCOL_VERSION}, keyremap speaks {protocol}"),
            },
        )?;
        return Ok(());
    }
    match role {
        Role::Status => protocol::write_message(&mut writer, &ToDaemon::Status(shared.status()))?,
        Role::Session => {
            *generation += 1;
            let session = *generation;
            protocol::write_message(&mut writer, &ToDaemon::Welcome { protocol: PROTOCOL_VERSION })?;
            writer.set_read_timeout(None)?;
            shared.open_session(session, writer);
            let shared = Arc::clone(shared);
            std::thread::Builder::new()
                .name(format!("keyremap-helper-session-{session}"))
                .spawn(move || read_session(session, lines, &shared))?;
        }
    }
    Ok(())
}

fn read_session(generation: u64, lines: impl Iterator<Item = io::Result<String>>, shared: &Shared) {
    for line in lines {
        let Ok(line) = line else {
            break;
        };
        match protocol::parse_message::<ToHelper>(&line) {
            Ok(ToHelper::Heartbeat) => shared.heartbeat(generation),
            Ok(ToHelper::Emit { usage_page, usage, pressed }) => {
                shared.emit(generation, usage_page, usage, pressed);
            }
            Ok(ToHelper::CapsLockLight { on }) => shared.set_caps_light(generation, on),
            Ok(ToHelper::Hello { .. }) => {}
            Err(error) => log::warn!("ignoring a bad message from keyremap: {error}"),
        }
    }
    shared.close_session(generation);
}

fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    let (mut uid, mut gid) = (0, 0);
    if unsafe { getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(uid)
}
```

The read timeout set on `writer` also clears it for the reader, because both handles share one socket.

- [ ] **Step 5: Implement the devices**

`devices.rs`. The manager is used for matching only and is never opened, so no device is opened until the watchdog asks for a seize.

```rust
use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr;
use std::sync::Arc;

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFRelease, CFRetain, CFType, CFTypeRef, TCFType};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::runloop::{
    kCFRunLoopDefaultMode, CFRunLoop, CFRunLoopRef, CFRunLoopTimer, CFRunLoopTimerContext,
    CFRunLoopTimerRef,
};
use core_foundation::string::{CFString, CFStringRef};

use super::protocol::{
    PAGE_APPLE_VENDOR_KEYBOARD, PAGE_APPLE_VENDOR_TOP_CASE, PAGE_CONSUMER, PAGE_KEYBOARD,
};
use super::report::PressedKeys;
use super::watchdog::Action;
use super::Shared;
use crate::platform::macos::virtual_hid::client::request::{
    VIRTUAL_KEYBOARD_PRODUCT_ID, VIRTUAL_KEYBOARD_VENDOR_ID,
};

type IOHIDManagerRef = *mut c_void;
type IOHIDDeviceRef = *mut c_void;
type IOHIDValueRef = *mut c_void;
type IOHIDElementRef = *mut c_void;
type IOReturn = i32;
type DeviceCallback = extern "C" fn(*mut c_void, IOReturn, *mut c_void, IOHIDDeviceRef);
type ValueCallback = extern "C" fn(*mut c_void, IOReturn, *mut c_void, IOHIDValueRef);

const OPTIONS_NONE: u32 = 0;
const OPTIONS_SEIZE: u32 = 1;
const RETURN_EXCLUSIVE_ACCESS: IOReturn = 0xE000_02C5_u32 as i32;
const REQUEST_LISTEN_EVENT: u32 = 1;
const ACCESS_GRANTED: u32 = 0;
const APPLE_VENDOR_ID: i64 = 0x05AC;
const TICK_SECONDS: f64 = 0.025;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOHIDManagerCreate(allocator: *const c_void, options: u32) -> IOHIDManagerRef;
    fn IOHIDManagerSetDeviceMatching(manager: IOHIDManagerRef, matching: CFDictionaryRef);
    fn IOHIDManagerRegisterDeviceMatchingCallback(manager: IOHIDManagerRef, callback: DeviceCallback, context: *mut c_void);
    fn IOHIDManagerRegisterDeviceRemovalCallback(manager: IOHIDManagerRef, callback: DeviceCallback, context: *mut c_void);
    fn IOHIDManagerScheduleWithRunLoop(manager: IOHIDManagerRef, run_loop: CFRunLoopRef, mode: CFStringRef);
    fn IOHIDDeviceOpen(device: IOHIDDeviceRef, options: u32) -> IOReturn;
    fn IOHIDDeviceClose(device: IOHIDDeviceRef, options: u32) -> IOReturn;
    fn IOHIDDeviceGetProperty(device: IOHIDDeviceRef, key: CFStringRef) -> CFTypeRef;
    fn IOHIDDeviceRegisterInputValueCallback(device: IOHIDDeviceRef, callback: Option<ValueCallback>, context: *mut c_void);
    fn IOHIDDeviceScheduleWithRunLoop(device: IOHIDDeviceRef, run_loop: CFRunLoopRef, mode: CFStringRef);
    fn IOHIDDeviceUnscheduleFromRunLoop(device: IOHIDDeviceRef, run_loop: CFRunLoopRef, mode: CFStringRef);
    fn IOHIDDeviceCopyMatchingElements(device: IOHIDDeviceRef, matching: CFDictionaryRef, options: u32) -> CFArrayRef;
    fn IOHIDDeviceSetValue(device: IOHIDDeviceRef, element: IOHIDElementRef, value: IOHIDValueRef) -> IOReturn;
    fn IOHIDValueCreateWithIntegerValue(allocator: *const c_void, element: IOHIDElementRef, timestamp: u64, value: isize) -> IOHIDValueRef;
    fn IOHIDValueGetElement(value: IOHIDValueRef) -> IOHIDElementRef;
    fn IOHIDValueGetIntegerValue(value: IOHIDValueRef) -> isize;
    fn IOHIDElementGetUsagePage(element: IOHIDElementRef) -> u32;
    fn IOHIDElementGetUsage(element: IOHIDElementRef) -> u32;
    fn IOHIDCheckAccess(request: u32) -> u32;
    fn IOHIDRequestAccess(request: u32) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFAbsoluteTimeGetCurrent() -> f64;
}

struct Keyboard {
    device: IOHIDDeviceRef,
    name: String,
    apple: bool,
    seized: bool,
    pressed: PressedKeys,
}

struct Devices {
    shared: Arc<Shared>,
    keyboards: RefCell<Vec<Keyboard>>,
    run_loop: CFRunLoop,
}

pub(super) fn input_monitoring_granted() -> bool {
    unsafe { IOHIDCheckAccess(REQUEST_LISTEN_EVENT) == ACCESS_GRANTED }
}

pub(super) fn request_input_monitoring() {
    if !input_monitoring_granted() && !unsafe { IOHIDRequestAccess(REQUEST_LISTEN_EVENT) } {
        log::warn!(
            "Input Monitoring is off for the keyboard helper; turn on com.qol-tools.keyremap.hid-helper in System Settings > Privacy & Security > Input Monitoring"
        );
    }
}

pub(super) fn run(shared: Arc<Shared>) -> ! {
    let devices: &'static Devices = Box::leak(Box::new(Devices {
        shared,
        keyboards: RefCell::new(Vec::new()),
        run_loop: CFRunLoop::get_current(),
    }));
    let context = ptr::from_ref(devices).cast_mut().cast::<c_void>();
    let matching = number_dictionary(&[("DeviceUsagePage", 0x01), ("DeviceUsage", 0x06)]);
    unsafe {
        let manager = IOHIDManagerCreate(ptr::null(), OPTIONS_NONE);
        IOHIDManagerSetDeviceMatching(manager, matching.as_concrete_TypeRef());
        IOHIDManagerRegisterDeviceMatchingCallback(manager, device_matched, context);
        IOHIDManagerRegisterDeviceRemovalCallback(manager, device_removed, context);
        IOHIDManagerScheduleWithRunLoop(manager, devices.run_loop.as_concrete_TypeRef(), kCFRunLoopDefaultMode);
    }
    let mut timer_context = CFRunLoopTimerContext {
        version: 0,
        info: context,
        retain: None,
        release: None,
        copyDescription: None,
    };
    let timer = CFRunLoopTimer::new(
        unsafe { CFAbsoluteTimeGetCurrent() } + TICK_SECONDS,
        TICK_SECONDS,
        0,
        0,
        tick,
        &mut timer_context,
    );
    devices.run_loop.add_timer(&timer, unsafe { kCFRunLoopDefaultMode });
    loop {
        CFRunLoop::run_current();
    }
}

extern "C" fn tick(_timer: CFRunLoopTimerRef, info: *mut c_void) {
    let devices = unsafe { &*info.cast::<Devices>() };
    let (action, caps_light) = devices.shared.tick();
    match action {
        Action::Seize => {
            let any = devices.seize_all();
            devices.shared.announce_seized(any);
        }
        Action::Release => {
            devices.release_all();
            devices.shared.announce_seized(false);
        }
        Action::Hold => {}
    }
    if let Some(on) = caps_light {
        devices.set_caps_lock_light(on);
    }
}

extern "C" fn device_matched(context: *mut c_void, _result: IOReturn, _sender: *mut c_void, device: IOHIDDeviceRef) {
    let devices = unsafe { &*context.cast::<Devices>() };
    let vendor = number_property(device, "VendorID").unwrap_or(0);
    let product = number_property(device, "ProductID").unwrap_or(0);
    let name = string_property(device, "Product").unwrap_or_else(|| format!("keyboard {vendor:04x}:{product:04x}"));
    if is_virtual_keyboard(vendor, product, &name) {
        return;
    }
    unsafe { CFRetain(device as CFTypeRef) };
    log::info!("keyboard arrived: {name}");
    devices.keyboards.borrow_mut().push(Keyboard {
        device,
        name,
        apple: vendor == APPLE_VENDOR_ID,
        seized: false,
        pressed: PressedKeys::default(),
    });
    let country_code = number_property(device, "CountryCode").unwrap_or(0);
    devices.shared.device_arrived(u64::try_from(country_code).unwrap_or(0));
}

extern "C" fn device_removed(context: *mut c_void, _result: IOReturn, _sender: *mut c_void, device: IOHIDDeviceRef) {
    let devices = unsafe { &*context.cast::<Devices>() };
    let removed = {
        let mut keyboards = devices.keyboards.borrow_mut();
        let Some(index) = keyboards.iter().position(|keyboard| keyboard.device == device) else {
            return;
        };
        keyboards.remove(index)
    };
    let mut removed = removed;
    for (page, usage) in removed.pressed.drain() {
        devices.shared.forward(page, usage, false, removed.apple);
    }
    if removed.seized {
        devices.close(&removed);
    }
    log::info!("keyboard removed: {}", removed.name);
    unsafe { CFRelease(removed.device as CFTypeRef) };
    devices.publish(Vec::new());
}

extern "C" fn value_changed(context: *mut c_void, _result: IOReturn, sender: *mut c_void, value: IOHIDValueRef) {
    let devices = unsafe { &*context.cast::<Devices>() };
    let element = unsafe { IOHIDValueGetElement(value) };
    let (Ok(page), Ok(usage)) = (
        u16::try_from(unsafe { IOHIDElementGetUsagePage(element) }),
        u16::try_from(unsafe { IOHIDElementGetUsage(element) }),
    ) else {
        return;
    };
    if !forwarded(page, usage) {
        return;
    }
    let pressed = unsafe { IOHIDValueGetIntegerValue(value) } != 0;
    let apple = {
        let mut keyboards = devices.keyboards.borrow_mut();
        let Some(keyboard) = keyboards.iter_mut().find(|keyboard| keyboard.device == sender) else {
            return;
        };
        keyboard.pressed.record(page, usage, pressed);
        keyboard.apple
    };
    devices.shared.forward(page, usage, pressed, apple);
}

impl Devices {
    fn seize_all(&self) -> bool {
        let mut conflicts = Vec::new();
        let context = ptr::from_ref(self).cast_mut().cast::<c_void>();
        for keyboard in self.keyboards.borrow_mut().iter_mut().filter(|keyboard| !keyboard.seized) {
            match unsafe { IOHIDDeviceOpen(keyboard.device, OPTIONS_SEIZE) } {
                0 => unsafe {
                    IOHIDDeviceRegisterInputValueCallback(keyboard.device, Some(value_changed), context);
                    IOHIDDeviceScheduleWithRunLoop(keyboard.device, self.run_loop.as_concrete_TypeRef(), kCFRunLoopDefaultMode);
                    keyboard.seized = true;
                    log::info!("seized {}", keyboard.name);
                },
                RETURN_EXCLUSIVE_ACCESS => {
                    log::warn!("{} is held by another app; leaving it alone", keyboard.name);
                    conflicts.push(keyboard.name.clone());
                }
                code => log::warn!("could not seize {}: IOReturn {code:#x}", keyboard.name),
            }
        }
        self.publish(conflicts)
    }

    fn release_all(&self) {
        for keyboard in self.keyboards.borrow_mut().iter_mut().filter(|keyboard| keyboard.seized) {
            self.close(keyboard);
            keyboard.seized = false;
            keyboard.pressed.drain();
            log::info!("released {}", keyboard.name);
        }
        self.publish(Vec::new());
    }

    fn close(&self, keyboard: &Keyboard) {
        unsafe {
            IOHIDDeviceRegisterInputValueCallback(keyboard.device, None, ptr::null_mut());
            IOHIDDeviceUnscheduleFromRunLoop(keyboard.device, self.run_loop.as_concrete_TypeRef(), kCFRunLoopDefaultMode);
            IOHIDDeviceClose(keyboard.device, OPTIONS_NONE);
        }
    }

    fn publish(&self, conflicts: Vec<String>) -> bool {
        let seized: Vec<String> = self
            .keyboards
            .borrow()
            .iter()
            .filter(|keyboard| keyboard.seized)
            .map(|keyboard| keyboard.name.clone())
            .collect();
        let any = !seized.is_empty();
        self.shared.publish_devices(seized, conflicts);
        any
    }

    fn set_caps_lock_light(&self, on: bool) {
        let matching = number_dictionary(&[("UsagePage", 0x08), ("Usage", 0x02)]);
        for keyboard in self.keyboards.borrow().iter().filter(|keyboard| keyboard.seized) {
            let elements = unsafe {
                IOHIDDeviceCopyMatchingElements(keyboard.device, matching.as_concrete_TypeRef(), OPTIONS_NONE)
            };
            if elements.is_null() {
                continue;
            }
            let elements: CFArray<CFType> = unsafe { CFArray::wrap_under_create_rule(elements) };
            for element in elements.iter() {
                let element = element.as_CFTypeRef() as IOHIDElementRef;
                let value = unsafe { IOHIDValueCreateWithIntegerValue(ptr::null(), element, 0, isize::from(on)) };
                if value.is_null() {
                    continue;
                }
                unsafe {
                    IOHIDDeviceSetValue(keyboard.device, element, value);
                    CFRelease(value as CFTypeRef);
                }
            }
        }
    }
}

fn forwarded(page: u16, usage: u16) -> bool {
    match page {
        PAGE_KEYBOARD => (0x04..=0xE7).contains(&usage),
        PAGE_CONSUMER | PAGE_APPLE_VENDOR_KEYBOARD | PAGE_APPLE_VENDOR_TOP_CASE => usage != 0,
        _ => false,
    }
}

fn is_virtual_keyboard(vendor: i64, product: i64, name: &str) -> bool {
    let pqrs = u64::try_from(vendor) == Ok(VIRTUAL_KEYBOARD_VENDOR_ID)
        && u64::try_from(product) == Ok(VIRTUAL_KEYBOARD_PRODUCT_ID);
    pqrs || name.contains("Karabiner")
}

fn number_dictionary(pairs: &[(&str, i64)]) -> CFDictionary<CFType, CFType> {
    let pairs: Vec<(CFType, CFType)> = pairs
        .iter()
        .map(|(key, value)| (CFString::new(key).as_CFType(), CFNumber::from(*value).as_CFType()))
        .collect();
    CFDictionary::from_CFType_pairs(&pairs)
}

fn property(device: IOHIDDeviceRef, key: &str) -> Option<CFType> {
    let key = CFString::new(key);
    let value = unsafe { IOHIDDeviceGetProperty(device, key.as_concrete_TypeRef()) };
    (!value.is_null()).then(|| unsafe { CFType::wrap_under_get_rule(value) })
}

fn number_property(device: IOHIDDeviceRef, key: &str) -> Option<i64> {
    property(device, key)?.downcast::<CFNumber>()?.to_i64()
}

fn string_property(device: IOHIDDeviceRef, key: &str) -> Option<String> {
    property(device, key)?.downcast::<CFString>().map(|name| name.to_string())
}
```

- [ ] **Step 6: Wire the command through the adapter**

`platform/mod.rs`, add to the trait:

```rust
    fn hid_helper(&self) -> Result<CommandResult>;
```

`platform/macos/mod.rs`, add to the impl:

```rust
    fn hid_helper(&self) -> Result<CommandResult> {
        hid_helper::run()?;
        Ok(CommandResult::success(""))
    }
```

`linux/mod.rs`, `windows/mod.rs`, `fallback/mod.rs`, each:

```rust
    fn hid_helper(&self) -> Result<CommandResult> {
        Ok(unsupported())
    }
```

`cli.rs`, in `app_with_handlers` add `let helper = adapter.clone();` beside the other clones and this command after `kill`:

```rust
        .command(
            Command::new("hid-helper")
                .about("Run the root keyboard helper that launchd starts.")
                .usage(format!("sudo {PLUGIN_ID} hid-helper"))
                .detail("Seizes the physical keyboards while Key Remap is running and feeds the virtual keyboard.")
                .detail("Releases every keyboard within 200 ms when Key Remap stops answering.")
                .output("Lifecycle diagnostics on stderr.")
                .exit_behavior("Runs until killed; exits non-zero when not root or not on macOS.")
                .run_result(move |context| {
                    no_args(context)?;
                    helper.hid_helper()
                }),
        )
```

In the cli tests add `hid_helper: AtomicUsize` to `Calls`, implement `fn hid_helper` on `SentinelAdapter` by counting it, add `vec!["help", "hid-helper"]` and `vec!["hid-helper", "help"]` to `doctor_and_help_never_reach_operational_paths`, and assert `calls.hid_helper` stays 0 there.

- [ ] **Step 7: Run the gate**

Run: `cargo test -p qol-keyremap && cargo clippy -p qol-keyremap --all-targets -- -D warnings`, then the Linux compile check from Global Constraints.
Expected: PASS. The Linux check proves the stub adapters compile.

- [ ] **Step 8: Check that matching works without opening the manager**

Run: `cargo build -p qol-keyremap && sudo ./target/debug/qol-keyremap hid-helper` for five seconds, then Ctrl+C.
Expected on stderr: one `keyboard arrived:` line per physical keyboard, `listening on /var/run/com.qol-tools.keyremap.hid-helper.sock`, and, with the pqrs daemon running, `virtual keyboard ready: true`. Nothing is seized, because no daemon session exists. If no `keyboard arrived:` line appears, add `IOHIDManagerOpen(manager, OPTIONS_NONE)` after scheduling (declared as `fn IOHIDManagerOpen(manager: IOHIDManagerRef, options: u32) -> IOReturn;`) and re-run; record which variant worked in the commit body.

- [ ] **Step 9: Commit**

```bash
git add plugins/keyremap/src
git commit -m "feat(keyremap): add the root keyboard helper"
```

---

### Task 9: Install and uninstall the helper

**Files:**
- Create: `plugins/keyremap/src/platform/macos/hid_helper/install.rs`
- Modify: `plugins/keyremap/src/platform/macos/hid_helper/mod.rs` (add `pub(crate) mod install;`)
- Modify: `plugins/keyremap/src/platform/mod.rs`, the four adapters, `plugins/keyremap/src/cli.rs`

**Interfaces:**
- Consumes: `virtual_hid::{PQRS_DAEMON_BINARY, PQRS_MANAGER_BINARY, PQRS_DAEMON_LABEL, OWN_DAEMON_LABEL}`, `protocol::SOCKET_PATH`, `hid_helper::is_root`.
- Produces:
  - `install::{HELPER_LABEL, HELPER_BINARY, HELPER_PLIST, OWN_DAEMON_PLIST}`
  - `install::install() -> anyhow::Result<String>`, `install::uninstall() -> anyhow::Result<String>` (the string is the human summary, one step per line)
  - `install::helper_plist() -> String`, `install::daemon_plist() -> String`
  - `PlatformAdapter::{install_hid_helper, uninstall_hid_helper}(&self) -> Result<CommandResult>`
  - CLI commands `install-hid-helper`, `uninstall-hid-helper`.

- [ ] **Step 1: Write the failing tests**

`install.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn lint(contents: &str) {
        let path = std::env::temp_dir().join(format!("qol-keyremap-plist-{}.plist", std::process::id()));
        std::fs::write(&path, contents).unwrap();
        let status = std::process::Command::new("/usr/bin/plutil").arg("-lint").arg(&path).status().unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(status.success(), "plutil rejected:\n{contents}");
    }

    #[test]
    fn the_helper_plist_runs_the_root_copy_as_hid_helper() {
        let plist = helper_plist();
        lint(&plist);
        assert!(plist.contains("<string>com.qol-tools.keyremap.hid-helper</string>"));
        assert!(plist.contains(&format!("<string>{HELPER_BINARY}</string>\n        <string>hid-helper</string>")));
        assert!(plist.contains("<key>KeepAlive</key>"));
    }

    #[test]
    fn the_daemon_plist_runs_the_pqrs_daemon_under_our_label() {
        let plist = daemon_plist();
        lint(&plist);
        assert!(plist.contains("<string>com.qol-tools.keyremap.vhid-daemon</string>"));
        assert!(plist.contains(PQRS_DAEMON_BINARY));
    }

    #[test]
    fn installing_without_root_names_sudo() {
        let error = install().unwrap_err().to_string();
        assert!(error.contains("sudo"), "{error}");
        let error = uninstall().unwrap_err().to_string();
        assert!(error.contains("sudo"), "{error}");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p qol-keyremap install`
Expected: compile error, `cannot find function helper_plist`.

- [ ] **Step 3: Implement**

`install.rs`:

```rust
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};

use super::protocol::SOCKET_PATH;
use crate::platform::macos::virtual_hid::{
    OWN_DAEMON_LABEL, PQRS_DAEMON_BINARY, PQRS_DAEMON_LABEL, PQRS_MANAGER_BINARY,
};

pub(crate) const HELPER_LABEL: &str = "com.qol-tools.keyremap.hid-helper";
pub(crate) const HELPER_BINARY: &str = "/Library/PrivilegedHelperTools/com.qol-tools.keyremap.hid-helper";
pub(crate) const HELPER_PLIST: &str = "/Library/LaunchDaemons/com.qol-tools.keyremap.hid-helper.plist";
pub(crate) const OWN_DAEMON_PLIST: &str = "/Library/LaunchDaemons/com.qol-tools.keyremap.vhid-daemon.plist";
const HELPER_LOG: &str = "/var/log/com.qol-tools.keyremap.hid-helper.log";
const LAUNCHCTL: &str = "/bin/launchctl";
const BOOTSTRAP_ATTEMPTS: u32 = 5;

pub(crate) fn install() -> Result<String> {
    ensure_root("install-hid-helper")?;
    ensure!(
        Path::new(PQRS_DAEMON_BINARY).exists(),
        "the virtual keyboard driver is not installed; install Karabiner-DriverKit-VirtualHIDDevice first"
    );
    let mut steps = Vec::new();
    let current = std::env::current_exe().context("find the running qol-keyremap binary")?;
    install_file(&current, HELPER_BINARY, 0o755)?;
    steps.push(format!("Copied {} to {HELPER_BINARY}.", current.display()));
    write_file(HELPER_PLIST, &helper_plist(), 0o644)?;
    if launchd_loaded(PQRS_DAEMON_LABEL) {
        steps.push(format!("Left {PQRS_DAEMON_LABEL} running the pqrs daemon."));
    } else {
        write_file(OWN_DAEMON_PLIST, &daemon_plist(), 0o644)?;
        reload(OWN_DAEMON_LABEL, OWN_DAEMON_PLIST)?;
        steps.push(format!("Started the pqrs daemon as {OWN_DAEMON_LABEL}."));
    }
    reload(HELPER_LABEL, HELPER_PLIST)?;
    steps.push(format!("Started {HELPER_LABEL}."));
    run(PQRS_MANAGER_BINARY, &["activate"])?;
    steps.push("Activated the driver extension.".to_string());
    steps.push(format!(
        "If macOS asks, turn on {HELPER_LABEL} in System Settings > Privacy & Security > Input Monitoring."
    ));
    Ok(steps.join("\n"))
}

pub(crate) fn uninstall() -> Result<String> {
    ensure_root("uninstall-hid-helper")?;
    let mut steps = Vec::new();
    bootout(HELPER_LABEL);
    for path in [HELPER_PLIST, HELPER_BINARY, SOCKET_PATH] {
        remove_if_present(path)?;
    }
    steps.push(format!("Stopped and removed {HELPER_LABEL}."));
    if Path::new(OWN_DAEMON_PLIST).exists() {
        bootout(OWN_DAEMON_LABEL);
        remove_if_present(OWN_DAEMON_PLIST)?;
        steps.push(format!("Stopped and removed {OWN_DAEMON_LABEL}."));
    }
    steps.push("Left the driver package installed.".to_string());
    Ok(steps.join("\n"))
}

pub(crate) fn launchd_loaded(label: &str) -> bool {
    Command::new(LAUNCHCTL)
        .args(["print", &format!("system/{label}")])
        .output()
        .is_ok_and(|output| output.status.success())
}

pub(crate) fn helper_plist() -> String {
    plist(HELPER_LABEL, &[HELPER_BINARY, "hid-helper"], Some(HELPER_LOG))
}

pub(crate) fn daemon_plist() -> String {
    plist(OWN_DAEMON_LABEL, &[PQRS_DAEMON_BINARY], None)
}

fn plist(label: &str, arguments: &[&str], log: Option<&str>) -> String {
    let arguments: String = arguments
        .iter()
        .map(|argument| format!("        <string>{argument}</string>\n"))
        .collect();
    let log = log.map_or(String::new(), |path| {
        format!("    <key>StandardErrorPath</key>\n    <string>{path}</string>\n")
    });
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
    <key>Label</key>\n\
    <string>{label}</string>\n\
    <key>ProgramArguments</key>\n\
    <array>\n\
{arguments}    </array>\n\
    <key>RunAtLoad</key>\n\
    <true/>\n\
    <key>KeepAlive</key>\n\
    <true/>\n\
    <key>ProcessType</key>\n\
    <string>Interactive</string>\n\
{log}</dict>\n\
</plist>\n"
    )
}

fn ensure_root(command: &str) -> Result<()> {
    if !super::is_root() {
        bail!("{command} changes system files; run it with sudo: sudo qol-keyremap {command}");
    }
    Ok(())
}

fn install_file(source: &Path, target: &str, mode: u32) -> Result<()> {
    let staging = format!("{target}.new");
    fs::copy(source, &staging).with_context(|| format!("copy to {staging}"))?;
    finish_file(&staging, target, mode)
}

fn write_file(target: &str, contents: &str, mode: u32) -> Result<()> {
    let staging = format!("{target}.new");
    fs::write(&staging, contents).with_context(|| format!("write {staging}"))?;
    finish_file(&staging, target, mode)
}

fn finish_file(staging: &str, target: &str, mode: u32) -> Result<()> {
    std::os::unix::fs::chown(staging, Some(0), Some(0)).with_context(|| format!("chown {staging}"))?;
    fs::set_permissions(staging, fs::Permissions::from_mode(mode)).with_context(|| format!("chmod {staging}"))?;
    fs::rename(staging, target).with_context(|| format!("move {staging} to {target}"))
}

fn remove_if_present(path: &str) -> Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("remove {path}"))
        }
        _ => Ok(()),
    }
}

fn reload(label: &str, plist: &str) -> Result<()> {
    bootout(label);
    let mut last = Ok(());
    for _ in 0..BOOTSTRAP_ATTEMPTS {
        last = run(LAUNCHCTL, &["bootstrap", "system", plist]);
        if last.is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    last
}

fn bootout(label: &str) {
    let _ = Command::new(LAUNCHCTL)
        .args(["bootout", &format!("system/{label}")])
        .output();
}

fn run(program: &str, arguments: &[&str]) -> Result<()> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .with_context(|| format!("run {program}"))?;
    if !output.status.success() {
        bail!(
            "{program} {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}
```

`platform/mod.rs` trait gains:

```rust
    fn install_hid_helper(&self) -> Result<CommandResult>;
    fn uninstall_hid_helper(&self) -> Result<CommandResult>;
```

macOS adapter:

```rust
    fn install_hid_helper(&self) -> Result<CommandResult> {
        Ok(summary_result(hid_helper::install::install()))
    }

    fn uninstall_hid_helper(&self) -> Result<CommandResult> {
        Ok(summary_result(hid_helper::install::uninstall()))
    }
```

and below `action_result`:

```rust
fn summary_result(result: Result<String>) -> CommandResult {
    match result {
        Ok(summary) => CommandResult::success(format!("{summary}\n")),
        Err(error) => CommandResult::runtime_error(format!("keyremap: {error:#}")),
    }
}
```

Linux, Windows and fallback adapters: both methods return `Ok(unsupported())`.

`cli.rs`: two more clones (`let install = adapter.clone(); let uninstall = adapter.clone();`) and two commands after `hid-helper`:

```rust
        .command(
            Command::new("install-hid-helper")
                .about("Install the root keyboard helper so Key Remap survives Secure Input.")
                .usage(format!("sudo {PLUGIN_ID} install-hid-helper"))
                .detail("Copies this binary to /Library/PrivilegedHelperTools and loads a LaunchDaemon for it.")
                .detail("Starts the pqrs virtual keyboard daemon if nothing else runs it, and activates the driver.")
                .output("One line per completed step on stdout.")
                .exit_behavior("Exits non-zero without sudo, without the driver package, or when launchctl fails.")
                .run_result(move |context| {
                    no_args(context)?;
                    install.install_hid_helper()
                }),
        )
        .command(
            Command::new("uninstall-hid-helper")
                .about("Remove the root keyboard helper; Key Remap falls back to the event tap.")
                .usage(format!("sudo {PLUGIN_ID} uninstall-hid-helper"))
                .detail("Leaves the driver package installed, since other software may use it.")
                .output("One line per completed step on stdout.")
                .exit_behavior("Exits non-zero without sudo or when a file cannot be removed.")
                .run_result(move |context| {
                    no_args(context)?;
                    uninstall.uninstall_hid_helper()
                }),
        )
```

Cli tests: add `install: AtomicUsize` and `uninstall: AtomicUsize` to `Calls`, count them in `SentinelAdapter`, add the four `help` orderings for both commands to `doctor_and_help_never_reach_operational_paths` and assert both counters stay 0.

- [ ] **Step 4: Run the gate**

Run: `cargo test -p qol-keyremap && cargo clippy -p qol-keyremap --all-targets -- -D warnings`, then the Linux compile check from Global Constraints.
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add plugins/keyremap/src
git commit -m "feat(keyremap): install the keyboard helper as a LaunchDaemon"
```

---

### Task 10: Key input strategies and hotkey markers

**Files:**
- Create: `plugins/keyremap/src/platform/macos/input/mod.rs`
- Create: `plugins/keyremap/src/platform/macos/input/backends/mod.rs`
- Create: `plugins/keyremap/src/platform/macos/input/backends/event_tap.rs`
- Modify: `plugins/keyremap/src/platform/macos/tap.rs`
- Modify: `plugins/keyremap/src/platform/macos/app/mod.rs`
- Modify: `plugins/keyremap/src/platform/macos/mod.rs` (add `#[allow(dead_code)] mod input;`)

**Interfaces:**
- Consumes: `remap::{Modifiers, KeyAction, process_key_event}`, `qol_runtime::keyremap_marker`.
- Produces:
  - `input::Strategy { EventTap, VirtualHid }`
  - `input::StrategyCell::default()`, `get(&self) -> Strategy`, `set(&self, Strategy) -> Strategy` (returns the previous one)
  - `input::MarkerBook::default()`, `insert(&self, keycode: u16, marker: i64)`, `release(&self, keycode: u16, now: Instant)`, `lookup(&self, keycode: u16, key_down: bool, now: Instant) -> Option<i64>`
  - `input::InputState { strategy: StrategyCell, markers: MarkerBook }` (`Default`)
  - `input::marker_for(mods: Modifiers, key: u16) -> i64`
  - `event_tap::{handle_key_event, stamp_marker, extract_modifiers, build_flags}`
  - `TapState::new(config, app_tracker, input: Arc<InputState>)`, `TapState::config(&self) -> Arc<ResolvedConfig>`, `TapState::input(&self) -> &Arc<InputState>`, `TapState::frontmost_bundle_id(&self) -> String`

This task moves code; the event tap must behave exactly as before under `Strategy::EventTap`, which is the only strategy anything selects until Task 12.

- [ ] **Step 1: Write the failing tests**

`input/mod.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn the_strategy_starts_on_the_event_tap_and_reports_the_previous_one() {
        let cell = StrategyCell::default();
        assert_eq!(cell.get(), Strategy::EventTap);
        assert_eq!(cell.set(Strategy::VirtualHid), Strategy::EventTap);
        assert_eq!(cell.get(), Strategy::VirtualHid);
        assert_eq!(cell.set(Strategy::EventTap), Strategy::VirtualHid);
    }

    #[test]
    fn a_marker_covers_the_held_key_and_its_late_key_up() {
        let book = MarkerBook::default();
        let start = Instant::now();
        book.insert(0x08, 42);
        assert_eq!(book.lookup(0x08, true, start), Some(42));
        book.release(0x08, start);
        assert_eq!(book.lookup(0x08, true, start), None, "a new key down is not the remapped one");
        assert_eq!(book.lookup(0x08, false, start + MARKER_GRACE), Some(42));
        assert_eq!(book.lookup(0x08, false, start + MARKER_GRACE + Duration::from_millis(1)), None);
        assert_eq!(book.lookup(0x09, true, start), None);
    }

    #[test]
    fn markers_encode_the_combo_the_user_pressed() {
        let ctrl = Modifiers { ctrl: true, ..Modifiers::NONE };
        let decoded = qol_runtime::keyremap_marker::decode(marker_for(ctrl, 0x08)).unwrap();
        assert_eq!(decoded.mods, qol_runtime::keyremap_marker::MOD_CTRL);
        assert_eq!(decoded.key, 0x08);

        let right_option = Modifiers { alt: true, ralt: true, ..Modifiers::NONE };
        let decoded = qol_runtime::keyremap_marker::decode(marker_for(right_option, 0x13)).unwrap();
        assert_eq!(decoded.mods, qol_runtime::keyremap_marker::MOD_ALT);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p qol-keyremap input::tests`
Expected: compile error, `cannot find type StrategyCell`.

- [ ] **Step 3: Implement `input/mod.rs`**

```rust
pub(crate) mod backends;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use qol_runtime::keyremap_marker;

use super::app::remap::Modifiers;

const MARKER_GRACE: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Strategy {
    EventTap,
    VirtualHid,
}

#[derive(Default)]
pub(crate) struct StrategyCell(AtomicU8);

impl StrategyCell {
    pub(crate) fn get(&self) -> Strategy {
        Self::decode(self.0.load(Ordering::SeqCst))
    }

    pub(crate) fn set(&self, strategy: Strategy) -> Strategy {
        Self::decode(self.0.swap(u8::from(strategy == Strategy::VirtualHid), Ordering::SeqCst))
    }

    fn decode(raw: u8) -> Strategy {
        if raw == 1 {
            Strategy::VirtualHid
        } else {
            Strategy::EventTap
        }
    }
}

#[derive(Default)]
pub(crate) struct MarkerBook {
    entries: Mutex<HashMap<u16, (i64, Option<Instant>)>>,
}

impl MarkerBook {
    pub(crate) fn insert(&self, keycode: u16, marker: i64) {
        self.entries().insert(keycode, (marker, None));
    }

    pub(crate) fn release(&self, keycode: u16, now: Instant) {
        if let Some(entry) = self.entries().get_mut(&keycode) {
            entry.1 = Some(now);
        }
    }

    pub(crate) fn lookup(&self, keycode: u16, key_down: bool, now: Instant) -> Option<i64> {
        let mut entries = self.entries();
        entries.retain(|_, (_, released)| {
            released.is_none_or(|at| now.saturating_duration_since(at) <= MARKER_GRACE)
        });
        let (marker, released) = entries.get(&keycode)?;
        (!key_down || released.is_none()).then_some(*marker)
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<u16, (i64, Option<Instant>)>> {
        self.entries.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[derive(Default)]
pub(crate) struct InputState {
    pub(crate) strategy: StrategyCell,
    pub(crate) markers: MarkerBook,
}

pub(crate) fn marker_for(mods: Modifiers, key: u16) -> i64 {
    let mut bits = 0;
    if mods.ctrl {
        bits |= keyremap_marker::MOD_CTRL;
    }
    if mods.shift {
        bits |= keyremap_marker::MOD_SHIFT;
    }
    if mods.alt {
        bits |= keyremap_marker::MOD_ALT;
    }
    if mods.cmd {
        bits |= keyremap_marker::MOD_SUPER;
    }
    keyremap_marker::encode(bits, key)
}
```

`input/backends/mod.rs`:

```rust
pub(crate) mod event_tap;
```

- [ ] **Step 4: Move the key handling out of `tap.rs`**

Create `input/backends/event_tap.rs` by moving these items from `tap.rs` unchanged: `TRACE_KEYS`, `event_character`, `tap_trace`, `handle_key_event`, `tag_remapped_key_event`, `strip_all_modifiers`, `NX_DEVICERALTKEYMASK`, `extract_modifiers`, `build_flags`, plus the imports they use. Delete `marker_mod_bits`; `tag_remapped_key_event` becomes:

```rust
fn tag_remapped_key_event(event: &CGEvent, original_mods: Modifiers, original_key: u16) {
    event.set_integer_value_field(
        EventField::EVENT_SOURCE_USER_DATA,
        marker_for(original_mods, original_key),
    );
}
```

Make `handle_key_event`, `extract_modifiers` and `build_flags` `pub(crate)`, and add:

```rust
pub(crate) fn stamp_marker(markers: &MarkerBook, event: &CGEvent, key_down: bool) -> CallbackResult {
    let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16;
    if let Some(marker) = markers.lookup(keycode, key_down, std::time::Instant::now()) {
        event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, marker);
    }
    CallbackResult::Keep
}
```

In `tap.rs`:

```rust
use super::input::backends::event_tap::{self, build_flags, extract_modifiers};
use super::input::{InputState, Strategy};

pub struct TapState {
    config: RwLock<Arc<ResolvedConfig>>,
    app_tracker: Arc<AppTracker>,
    input: Arc<InputState>,
}

impl TapState {
    pub fn new(config: ResolvedConfig, app_tracker: Arc<AppTracker>, input: Arc<InputState>) -> Self {
        Self {
            config: RwLock::new(Arc::new(config)),
            app_tracker,
            input,
        }
    }

    pub(crate) fn config(&self) -> Arc<ResolvedConfig> {
        self.config
            .read()
            .map(|g| g.clone())
            .unwrap_or_else(|p| p.into_inner().clone())
    }

    pub(crate) fn input(&self) -> &Arc<InputState> {
        &self.input
    }

    pub(crate) fn frontmost_bundle_id(&self) -> String {
        self.app_tracker.bundle_id_for_target(0)
    }
}
```

(keep `swap_config` as is) and the key arm of `handle_event` becomes:

```rust
        CGEventType::KeyDown | CGEventType::KeyUp => match state.input.strategy.get() {
            Strategy::EventTap => {
                event_tap::handle_key_event(config.as_ref(), event, target_pid, &bundle_id)
            }
            Strategy::VirtualHid => event_tap::stamp_marker(
                &state.input.markers,
                event,
                matches!(event_type, CGEventType::KeyDown),
            ),
        },
```

In `app/mod.rs`, construct the state with an input handle:

```rust
            let state = std::sync::Arc::new(super::tap::TapState::new(
                resolved,
                app_tracker,
                std::sync::Arc::new(super::input::InputState::default()),
            ));
```

- [ ] **Step 5: Run the gate**

Run: `cargo test -p qol-keyremap && cargo clippy -p qol-keyremap --all-targets -- -D warnings`
Expected: PASS. `git diff --stat` shows `tap.rs` shrinking by the moved lines and `event_tap.rs` gaining them.

- [ ] **Step 6: Check today's behaviour by hand**

Run `qol dev`, then in TextEdit press Ctrl+C on selected text and paste with Cmd+V, and type Right Option+2.
Expected: the copy works and `@` appears, exactly as before the change.

- [ ] **Step 7: Commit**

```bash
git add plugins/keyremap/src/platform/macos
git commit -m "refactor(keyremap): make key input a strategy with the event tap as one backend"
```

---

### Task 11: Keyboard state for the virtual HID strategy

**Files:**
- Create: `plugins/keyremap/src/platform/macos/input/backends/virtual_hid/mod.rs` (module declarations only in this task: `pub(crate) mod fn_keys; pub(crate) mod keyboard;`)
- Create: `plugins/keyremap/src/platform/macos/input/backends/virtual_hid/keyboard.rs`
- Create: `plugins/keyremap/src/platform/macos/input/backends/virtual_hid/fn_keys.rs`
- Modify: `plugins/keyremap/src/platform/macos/input/backends/mod.rs` (add `#[allow(dead_code)] pub(crate) mod virtual_hid;`)

**Interfaces:**
- Consumes: `remap::{process_key_event, KeyAction, Modifiers, ResolvedConfig}`, `macos_keycode::{from_hid_usage, to_hid_usage, PhysicalLayout}`, `CharTable`, `KeyStroke`, `marker_for`, `protocol::PAGE_*`.
- Produces:
  - `fn_keys::translate(usage: u16, fn_down: bool, fn_keys_are_standard: bool) -> Option<(u16, u16)>` (page, usage)
  - `keyboard::Emit { page: u16, usage: u16, pressed: bool }`
  - `keyboard::Output { Emit(Emit), Mark { keycode: u16, marker: i64 }, Unmark { keycode: u16 }, MissingChar(String), CapsLock }`
  - `keyboard::KeyContext<'a> { config: &'a ResolvedConfig, table: &'a CharTable, physical: PhysicalLayout, bundle_id: &'a str, fn_state: bool }`
  - `keyboard::KeyboardState::default()`, `handle(&mut self, page: u16, usage: u16, pressed: bool, apple: bool, context: &KeyContext<'_>) -> Vec<Output>`, `reset(&mut self)`
  - `keyboard::modifiers_from_bits(u8) -> Modifiers`

Behaviour this pins down, matching today's tap:
- A remapped key moves the emitted modifiers to the rule's target for as long as the key is held, then restores the physical ones.
- A character is typed with only the modifiers its key strokes need. Every stroke but the last is tapped; the last is held until the physical key comes up, so holding the key repeats a one-stroke character as it does today.
- Right Option rules still only match Right Option, because the physical bits keep the side.
- A key's release always undoes what its press did, even if Fn or the rules changed in between.

- [ ] **Step 1: Write the failing tests**

`fn_keys.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_top_row_is_media_unless_fn_or_the_setting_says_otherwise() {
        assert_eq!(translate(0x3A, false, false), Some((PAGE_CONSUMER, 0x70)));
        assert_eq!(translate(0x3A, true, false), None);
        assert_eq!(translate(0x3A, false, true), None);
        assert_eq!(translate(0x3A, true, true), Some((PAGE_CONSUMER, 0x70)));
        assert_eq!(translate(0x3C, false, false), Some((PAGE_APPLE_VENDOR_KEYBOARD, 0x10)));
        assert_eq!(translate(0x3D, false, false), Some((PAGE_APPLE_VENDOR_KEYBOARD, 0x01)));
        assert_eq!(translate(0x45, false, false), Some((PAGE_CONSUMER, 0xE9)));
    }

    #[test]
    fn f6_stays_f6() {
        assert_eq!(translate(0x3F, false, false), None);
        assert_eq!(translate(0x3F, true, false), None);
    }

    #[test]
    fn fn_with_arrows_backspace_and_return_navigates() {
        assert_eq!(translate(0x50, true, false), Some((PAGE_KEYBOARD, 0x4A)));
        assert_eq!(translate(0x4F, true, false), Some((PAGE_KEYBOARD, 0x4D)));
        assert_eq!(translate(0x52, true, false), Some((PAGE_KEYBOARD, 0x4B)));
        assert_eq!(translate(0x51, true, false), Some((PAGE_KEYBOARD, 0x4E)));
        assert_eq!(translate(0x2A, true, false), Some((PAGE_KEYBOARD, 0x4C)));
        assert_eq!(translate(0x28, true, false), Some((PAGE_KEYBOARD, 0x58)));
        assert_eq!(translate(0x50, false, false), None);
    }
}
```

`keyboard.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::platform::macos::app::config::RemapConfig;
    use crate::platform::macos::hid_helper::protocol::PAGE_CONSUMER;

    const OPTION: u32 = 0x08;
    const SHIFT: u32 = 0x02;

    fn table() -> CharTable {
        CharTable::from_translation(|code, state, dead| {
            if code == keycode::SPACE && state == 0 {
                return Some(if std::mem::take(dead) == 1 { "~" } else { " " }.to_string());
            }
            let text = match (code, state) {
                (0x2A, OPTION) => "@",
                (0x0A, 0) => "$",
                (0x15, SHIFT) => "€",
                (0x08, 0) => "c",
                (0x1E, OPTION) => {
                    *dead = 1;
                    ""
                }
                _ => return None,
            };
            Some(text.to_string())
        })
    }

    fn config(raw: serde_json::Value) -> ResolvedConfig {
        remap::resolve(&serde_json::from_value::<RemapConfig>(raw).unwrap())
    }

    fn rules() -> ResolvedConfig {
        config(json!({
            "excluded_apps": ["com.example.excluded"],
            "char_rules": [
                { "from_mods": ["ralt"], "from_key": "2", "to_char": "@", "global": true },
                { "from_mods": ["ralt"], "from_key": "rightbracket", "to_char": "~" },
                { "from_mods": ["ralt"], "from_key": "3", "to_char": "あ" }
            ],
            "key_rules": [
                { "from_mods": ["ctrl"], "from_key": "c", "to_mods": ["cmd"], "to_key": "c" }
            ]
        }))
    }

    struct Fixture {
        config: ResolvedConfig,
        table: CharTable,
        bundle: String,
        fn_state: bool,
        state: KeyboardState,
    }

    impl Fixture {
        fn new(config: ResolvedConfig) -> Self {
            Self { config, table: table(), bundle: "com.apple.TextEdit".to_string(), fn_state: false, state: KeyboardState::default() }
        }

        fn key(&mut self, page: u16, usage: u16, pressed: bool, apple: bool) -> Vec<Output> {
            let context = KeyContext {
                config: &self.config,
                table: &self.table,
                physical: PhysicalLayout::Iso,
                bundle_id: &self.bundle,
                fn_state: self.fn_state,
            };
            self.state.handle(page, usage, pressed, apple, &context)
        }

        fn press(&mut self, usage: u16) -> Vec<Output> {
            self.key(PAGE_KEYBOARD, usage, true, false)
        }

        fn release(&mut self, usage: u16) -> Vec<Output> {
            self.key(PAGE_KEYBOARD, usage, false, false)
        }
    }

    fn down(usage: u16) -> Output {
        Output::Emit(Emit { page: PAGE_KEYBOARD, usage, pressed: true })
    }

    fn up(usage: u16) -> Output {
        Output::Emit(Emit { page: PAGE_KEYBOARD, usage, pressed: false })
    }

    const CTRL: Modifiers = Modifiers { ctrl: true, ..Modifiers::NONE };
    const RIGHT_OPTION: Modifiers = Modifiers { alt: true, ralt: true, ..Modifiers::NONE };

    #[test]
    fn unmapped_keys_pass_through() {
        let mut fixture = Fixture::new(rules());
        assert_eq!(fixture.press(0x04), vec![down(0x04)]);
        assert_eq!(fixture.release(0x04), vec![up(0x04)]);
    }

    #[test]
    fn ctrl_c_becomes_cmd_c_and_ctrl_comes_back() {
        let mut fixture = Fixture::new(rules());
        assert_eq!(fixture.press(0xE0), vec![down(0xE0)]);
        assert_eq!(
            fixture.press(0x06),
            vec![
                up(0xE0),
                down(0xE3),
                Output::Mark { keycode: 0x08, marker: marker_for(CTRL, 0x08) },
                down(0x06),
            ]
        );
        assert_eq!(
            fixture.release(0x06),
            vec![up(0x06), Output::Unmark { keycode: 0x08 }, up(0xE3), down(0xE0)]
        );
        assert_eq!(fixture.release(0xE0), vec![up(0xE0)]);
    }

    #[test]
    fn right_option_2_types_at_with_left_option_only() {
        let mut fixture = Fixture::new(rules());
        assert_eq!(fixture.press(0xE6), vec![down(0xE6)]);
        assert_eq!(
            fixture.press(0x1F),
            vec![
                up(0xE6),
                down(0xE2),
                Output::Mark { keycode: 0x2A, marker: marker_for(RIGHT_OPTION, 0x13) },
                down(0x31),
            ]
        );
        assert_eq!(
            fixture.release(0x1F),
            vec![up(0x31), Output::Unmark { keycode: 0x2A }, up(0xE2), down(0xE6)]
        );
    }

    #[test]
    fn left_option_2_is_not_the_right_option_rule() {
        let mut fixture = Fixture::new(rules());
        assert_eq!(fixture.press(0xE2), vec![down(0xE2)]);
        assert_eq!(fixture.press(0x1F), vec![down(0x1F)]);
    }

    #[test]
    fn a_dead_key_character_taps_the_dead_key_then_holds_space() {
        let mut fixture = Fixture::new(rules());
        fixture.press(0xE6);
        let marker = marker_for(RIGHT_OPTION, 0x1E);
        assert_eq!(
            fixture.press(0x30),
            vec![
                up(0xE6),
                down(0xE2),
                Output::Mark { keycode: 0x1E, marker },
                down(0x30),
                up(0x30),
                Output::Unmark { keycode: 0x1E },
                up(0xE2),
                Output::Mark { keycode: keycode::SPACE, marker },
                down(0x2C),
            ]
        );
        assert_eq!(
            fixture.release(0x30),
            vec![up(0x2C), Output::Unmark { keycode: keycode::SPACE }, down(0xE6)]
        );
    }

    #[test]
    fn a_character_the_layout_lacks_is_reported_and_swallowed() {
        let mut fixture = Fixture::new(rules());
        fixture.press(0xE6);
        assert_eq!(fixture.press(0x20), vec![Output::MissingChar("あ".to_string())]);
        assert_eq!(fixture.release(0x20), Vec::new());
    }

    #[test]
    fn non_global_rules_skip_excluded_apps_and_global_ones_do_not() {
        let mut fixture = Fixture::new(rules());
        fixture.bundle = "com.example.excluded".to_string();
        fixture.press(0xE0);
        assert_eq!(fixture.press(0x06), vec![down(0x06)]);
        fixture.release(0x06);
        fixture.release(0xE0);
        fixture.press(0xE6);
        assert!(fixture.press(0x1F).contains(&down(0x31)));
    }

    #[test]
    fn a_disabled_config_passes_everything() {
        let mut fixture = Fixture::new(config(json!({
            "enabled": false,
            "key_rules": [{ "from_mods": ["ctrl"], "from_key": "c", "to_mods": ["cmd"], "to_key": "c" }]
        })));
        fixture.press(0xE0);
        assert_eq!(fixture.press(0x06), vec![down(0x06)]);
    }

    #[test]
    fn char_swaps_see_the_character_the_key_would_type() {
        let mut fixture = Fixture::new(config(json!({ "char_swaps": [["$", "€"]] })));
        assert_eq!(
            fixture.press(0x35),
            vec![
                down(0xE1),
                Output::Mark { keycode: 0x15, marker: marker_for(Modifiers::NONE, 0x0A) },
                down(0x21),
            ]
        );
        assert_eq!(
            fixture.release(0x35),
            vec![up(0x21), Output::Unmark { keycode: 0x15 }, up(0xE1)]
        );
    }

    #[test]
    fn the_apple_top_row_sends_media_keys_and_releases_them_after_fn_changes() {
        let mut fixture = Fixture::new(rules());
        assert_eq!(
            fixture.key(PAGE_KEYBOARD, 0x3A, true, true),
            vec![Output::Emit(Emit { page: PAGE_CONSUMER, usage: 0x70, pressed: true })]
        );
        fixture.key(PAGE_APPLE_VENDOR_TOP_CASE, 0x03, true, true);
        assert_eq!(
            fixture.key(PAGE_KEYBOARD, 0x3A, false, true),
            vec![Output::Emit(Emit { page: PAGE_CONSUMER, usage: 0x70, pressed: false })]
        );
        assert_eq!(fixture.key(PAGE_KEYBOARD, 0x3A, true, true), vec![down(0x3A)]);
        assert_eq!(fixture.key(PAGE_KEYBOARD, 0x3A, true, false), vec![down(0x3A)]);
    }

    #[test]
    fn caps_lock_asks_for_a_light_update() {
        let mut fixture = Fixture::new(rules());
        assert_eq!(fixture.press(0x39), vec![Output::CapsLock, down(0x39)]);
    }

    #[test]
    fn reset_forgets_held_keys() {
        let mut fixture = Fixture::new(rules());
        fixture.press(0xE0);
        fixture.press(0x06);
        fixture.state.reset();
        assert_eq!(fixture.release(0x06), vec![up(0x06)]);
        assert_eq!(fixture.release(0xE0), Vec::new());
    }

    #[test]
    fn modifier_bits_keep_the_side_of_option() {
        assert_eq!(modifiers_from_bits(0x40), RIGHT_OPTION);
        assert_eq!(modifiers_from_bits(0x04), Modifiers { alt: true, ..Modifiers::NONE });
        assert_eq!(modifiers_from_bits(0x88), Modifiers { cmd: true, ..Modifiers::NONE });
    }
}
```

`Modifiers` needs `const`-constructible struct update syntax; `Modifiers::NONE` is already a `pub const`, so `Modifiers { ctrl: true, ..Modifiers::NONE }` works in a `const`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p qol-keyremap virtual_hid::keyboard virtual_hid::fn_keys`
Expected: compile error, `cannot find type KeyboardState`.

- [ ] **Step 3: Implement**

`fn_keys.rs`:

```rust
use crate::platform::macos::hid_helper::protocol::{
    PAGE_APPLE_VENDOR_KEYBOARD, PAGE_CONSUMER, PAGE_KEYBOARD,
};

const TOP_ROW: [(u16, u16, u16); 11] = [
    (0x3A, PAGE_CONSUMER, 0x70),
    (0x3B, PAGE_CONSUMER, 0x6F),
    (0x3C, PAGE_APPLE_VENDOR_KEYBOARD, 0x10),
    (0x3D, PAGE_APPLE_VENDOR_KEYBOARD, 0x01),
    (0x3E, PAGE_CONSUMER, 0xCF),
    (0x40, PAGE_CONSUMER, 0xB4),
    (0x41, PAGE_CONSUMER, 0xCD),
    (0x42, PAGE_CONSUMER, 0xB3),
    (0x43, PAGE_CONSUMER, 0xE2),
    (0x44, PAGE_CONSUMER, 0xEA),
    (0x45, PAGE_CONSUMER, 0xE9),
];

const FN_NAVIGATION: [(u16, u16); 6] = [
    (0x50, 0x4A),
    (0x4F, 0x4D),
    (0x52, 0x4B),
    (0x51, 0x4E),
    (0x2A, 0x4C),
    (0x28, 0x58),
];

pub(crate) fn translate(usage: u16, fn_down: bool, fn_keys_are_standard: bool) -> Option<(u16, u16)> {
    if let Some(&(_, page, media)) = TOP_ROW.iter().find(|(key, ..)| *key == usage) {
        return (fn_down == fn_keys_are_standard).then_some((page, media));
    }
    if !fn_down {
        return None;
    }
    FN_NAVIGATION
        .iter()
        .find(|(key, _)| *key == usage)
        .map(|&(_, target)| (PAGE_KEYBOARD, target))
}
```

`keyboard.rs`:

```rust
use qol_hotkeys::macos_keycode::{self as keycode, PhysicalLayout};

use super::fn_keys;
use crate::platform::macos::app::remap::{self, KeyAction, Modifiers, ResolvedConfig};
use crate::platform::macos::hid_helper::protocol::{PAGE_APPLE_VENDOR_TOP_CASE, PAGE_KEYBOARD};
use crate::platform::macos::input::marker_for;
use crate::platform::macos::layout::{CharTable, KeyStroke};

const FN_USAGE: u16 = 0x03;
const CAPS_LOCK_USAGE: u16 = 0x39;
const FIRST_MODIFIER: u16 = 0xE0;
const LAST_MODIFIER: u16 = 0xE7;
const LEFT_SHIFT: u8 = 0x02;
const LEFT_OPTION: u8 = 0x04;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Emit {
    pub(crate) page: u16,
    pub(crate) usage: u16,
    pub(crate) pressed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Output {
    Emit(Emit),
    Mark { keycode: u16, marker: i64 },
    Unmark { keycode: u16 },
    MissingChar(String),
    CapsLock,
}

pub(crate) struct KeyContext<'a> {
    pub(crate) config: &'a ResolvedConfig,
    pub(crate) table: &'a CharTable,
    pub(crate) physical: PhysicalLayout,
    pub(crate) bundle_id: &'a str,
    pub(crate) fn_state: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    Pass { page: u16, usage: u16 },
    Output { usage: u16, keycode: u16, bits: u8 },
    Swallowed,
}

#[derive(Debug, Default)]
pub(crate) struct KeyboardState {
    physical_bits: u8,
    emitted_bits: u8,
    fn_down: bool,
    held: Vec<(u16, Held)>,
}

impl KeyboardState {
    pub(crate) fn handle(
        &mut self,
        page: u16,
        usage: u16,
        pressed: bool,
        apple: bool,
        context: &KeyContext<'_>,
    ) -> Vec<Output> {
        let mut outputs = Vec::new();
        match page {
            PAGE_KEYBOARD if (FIRST_MODIFIER..=LAST_MODIFIER).contains(&usage) => {
                self.modifier(usage, pressed, &mut outputs);
            }
            PAGE_KEYBOARD if pressed => self.press(usage, apple, context, &mut outputs),
            PAGE_KEYBOARD => self.release(usage, &mut outputs),
            PAGE_APPLE_VENDOR_TOP_CASE if usage == FN_USAGE => {
                self.fn_down = pressed;
                outputs.push(emit(page, usage, pressed));
            }
            _ => outputs.push(emit(page, usage, pressed)),
        }
        outputs
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    fn modifier(&mut self, usage: u16, pressed: bool, outputs: &mut Vec<Output>) {
        let bit = 1u8 << (usage - FIRST_MODIFIER);
        if pressed {
            self.physical_bits |= bit;
        } else {
            self.physical_bits &= !bit;
        }
        let bits = self.resting_bits();
        self.move_mods_to(bits, outputs);
    }

    fn press(&mut self, usage: u16, apple: bool, context: &KeyContext<'_>, outputs: &mut Vec<Output>) {
        if apple {
            if let Some((page, translated)) = fn_keys::translate(usage, self.fn_down, context.fn_state) {
                self.held.push((usage, Held::Pass { page, usage: translated }));
                outputs.push(emit(page, translated, true));
                return;
            }
        }
        if usage == CAPS_LOCK_USAGE {
            outputs.push(Output::CapsLock);
        }
        let Some(code) = keycode::from_hid_usage(usage, context.physical) else {
            return self.pass(usage, outputs);
        };
        let mods = modifiers_from_bits(self.physical_bits);
        let event_char = if context.config.char_swap_rules.is_empty() {
            None
        } else {
            context.table.char_at(code, mods.shift, mods.alt)
        };
        let action = if context.config.enabled {
            remap::process_key_event(context.config, mods, code, event_char, context.bundle_id)
        } else {
            KeyAction::Passthrough
        };
        match action {
            KeyAction::Passthrough => self.pass(usage, outputs),
            KeyAction::Remap { mods: target, key } => {
                let Some(out_usage) = keycode::to_hid_usage(key, context.physical) else {
                    return self.pass(usage, outputs);
                };
                let bits = target_bits(self.physical_bits, mods, target);
                self.move_mods_to(bits, outputs);
                outputs.push(Output::Mark { keycode: key, marker: marker_for(mods, code) });
                outputs.push(emit(PAGE_KEYBOARD, out_usage, true));
                self.held.push((usage, Held::Output { usage: out_usage, keycode: key, bits }));
            }
            KeyAction::Char { text } => {
                self.type_text(usage, &text, marker_for(mods, code), context, outputs);
            }
        }
    }

    fn type_text(
        &mut self,
        usage: u16,
        text: &str,
        marker: i64,
        context: &KeyContext<'_>,
        outputs: &mut Vec<Output>,
    ) {
        let strokes: Option<Vec<(KeyStroke, u16)>> = context.table.sequence_for(text).and_then(|strokes| {
            strokes
                .into_iter()
                .map(|stroke| keycode::to_hid_usage(stroke.keycode, context.physical).map(|out| (stroke, out)))
                .collect()
        });
        let Some((&(last, last_usage), rest)) = strokes.as_deref().and_then(<[_]>::split_last) else {
            outputs.push(Output::MissingChar(text.to_string()));
            self.held.push((usage, Held::Swallowed));
            return;
        };
        for &(stroke, stroke_usage) in rest {
            self.move_mods_to(stroke_bits(stroke), outputs);
            outputs.push(Output::Mark { keycode: stroke.keycode, marker });
            outputs.push(emit(PAGE_KEYBOARD, stroke_usage, true));
            outputs.push(emit(PAGE_KEYBOARD, stroke_usage, false));
            outputs.push(Output::Unmark { keycode: stroke.keycode });
        }
        let bits = stroke_bits(last);
        self.move_mods_to(bits, outputs);
        outputs.push(Output::Mark { keycode: last.keycode, marker });
        outputs.push(emit(PAGE_KEYBOARD, last_usage, true));
        self.held.push((usage, Held::Output { usage: last_usage, keycode: last.keycode, bits }));
    }

    fn pass(&mut self, usage: u16, outputs: &mut Vec<Output>) {
        self.held.push((usage, Held::Pass { page: PAGE_KEYBOARD, usage }));
        outputs.push(emit(PAGE_KEYBOARD, usage, true));
    }

    fn release(&mut self, usage: u16, outputs: &mut Vec<Output>) {
        let Some(index) = self.held.iter().rposition(|(held, _)| *held == usage) else {
            outputs.push(emit(PAGE_KEYBOARD, usage, false));
            return;
        };
        match self.held.remove(index).1 {
            Held::Pass { page, usage } => outputs.push(emit(page, usage, false)),
            Held::Output { usage, keycode, .. } => {
                outputs.push(emit(PAGE_KEYBOARD, usage, false));
                outputs.push(Output::Unmark { keycode });
                let bits = self.resting_bits();
                self.move_mods_to(bits, outputs);
            }
            Held::Swallowed => {}
        }
    }

    fn resting_bits(&self) -> u8 {
        self.held
            .iter()
            .rev()
            .find_map(|(_, held)| match held {
                Held::Output { bits, .. } => Some(*bits),
                _ => None,
            })
            .unwrap_or(self.physical_bits)
    }

    fn move_mods_to(&mut self, bits: u8, outputs: &mut Vec<Output>) {
        let changed = self.emitted_bits ^ bits;
        for pressed in [false, true] {
            for index in 0..8u16 {
                let bit = 1u8 << index;
                if changed & bit != 0 && (bits & bit != 0) == pressed {
                    outputs.push(emit(PAGE_KEYBOARD, FIRST_MODIFIER + index, pressed));
                }
            }
        }
        self.emitted_bits = bits;
    }
}

pub(crate) fn modifiers_from_bits(bits: u8) -> Modifiers {
    Modifiers {
        ctrl: bits & 0x11 != 0,
        shift: bits & 0x22 != 0,
        alt: bits & 0x44 != 0,
        cmd: bits & 0x88 != 0,
        ralt: bits & 0x40 != 0,
    }
}

fn target_bits(physical: u8, from: Modifiers, to: Modifiers) -> u8 {
    let mut bits = physical;
    for (was, wanted, both_sides, left) in [
        (from.ctrl, to.ctrl, 0x11, 0x01),
        (from.shift, to.shift, 0x22, 0x02),
        (from.alt, to.alt, 0x44, 0x04),
        (from.cmd, to.cmd, 0x88, 0x08),
    ] {
        if was && !wanted {
            bits &= !both_sides;
        }
        if !was && wanted {
            bits |= left;
        }
    }
    bits
}

fn stroke_bits(stroke: KeyStroke) -> u8 {
    let shift = if stroke.shift { LEFT_SHIFT } else { 0 };
    let option = if stroke.option { LEFT_OPTION } else { 0 };
    shift | option
}

fn emit(page: u16, usage: u16, pressed: bool) -> Output {
    Output::Emit(Emit { page, usage, pressed })
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p qol-keyremap virtual_hid`
Expected: PASS, 16 tests. If `char_swaps_see_the_character_the_key_would_type` fails on the usage for kVK 0x15, check Task 2's table: kVK 0x15 is `ANSI_4`, HID 0x21.

- [ ] **Step 5: Commit**

```bash
git add plugins/keyremap/src/platform/macos/input
git commit -m "feat(keyremap): turn helper key events into virtual keyboard presses"
```

---

### Task 12: The daemon session with the helper

**Files:**
- Modify: `plugins/keyremap/src/platform/macos/input/backends/virtual_hid/mod.rs`
- Modify: `plugins/keyremap/src/platform/macos/tap.rs` (implement `KeyEnvironment`)
- Modify: `plugins/keyremap/src/platform/macos/app/mod.rs`

**Interfaces:**
- Consumes: `KeyboardState`, `KeyContext`, `Output` (Task 11), `InputState`, `Strategy` (Task 10), `LayoutStore`, `LayoutSnapshot` (Task 4), `protocol::*` (Task 7).
- Produces:
  - `virtual_hid::KeyEnvironment` trait: `config(&self) -> Arc<ResolvedConfig>`, `frontmost_bundle_id(&self) -> String`, `input(&self) -> &Arc<InputState>`; implemented by `TapState`.
  - `virtual_hid::start(environment: Arc<impl KeyEnvironment + 'static>, layouts: Arc<LayoutStore>)`: background thread that keeps a session open and falls back to `Strategy::EventTap` whenever it is not.
  - `virtual_hid::run_session(stream: UnixStream, environment: &impl KeyEnvironment, layouts: &LayoutStore) -> anyhow::Result<()>`

- [ ] **Step 1: Write the failing tests**

Append to `virtual_hid/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;
    use std::time::Duration;

    use qol_hotkeys::macos_keycode::PhysicalLayout;
    use serde_json::json;

    use super::*;
    use crate::platform::macos::app::config::RemapConfig;
    use crate::platform::macos::app::remap;
    use crate::platform::macos::layout::{CharTable, LayoutSnapshot};

    struct FakeEnvironment {
        config: Arc<ResolvedConfig>,
        input: Arc<InputState>,
    }

    impl KeyEnvironment for FakeEnvironment {
        fn config(&self) -> Arc<ResolvedConfig> {
            Arc::clone(&self.config)
        }

        fn frontmost_bundle_id(&self) -> String {
            "com.apple.TextEdit".to_string()
        }

        fn input(&self) -> &Arc<InputState> {
            &self.input
        }
    }

    fn environment(enabled: bool) -> FakeEnvironment {
        let raw: RemapConfig = serde_json::from_value(json!({
            "enabled": enabled,
            "key_rules": [{ "from_mods": ["ctrl"], "from_key": "c", "to_mods": ["cmd"], "to_key": "c" }]
        }))
        .unwrap();
        FakeEnvironment { config: Arc::new(remap::resolve(&raw)), input: Arc::new(InputState::default()) }
    }

    fn layouts() -> LayoutStore {
        LayoutStore::new(LayoutSnapshot {
            id: "test".to_string(),
            table: CharTable::from_translation(|_, _, _| None),
            physical: PhysicalLayout::Ansi,
        })
    }

    struct FakeHelper {
        stream: UnixStream,
        lines: std::io::Lines<BufReader<UnixStream>>,
    }

    impl FakeHelper {
        fn say(&mut self, message: &ToDaemon) {
            protocol::write_message(&mut self.stream, message).unwrap();
        }

        fn next_non_heartbeat(&mut self) -> ToHelper {
            loop {
                let line = self.lines.next().expect("session closed").unwrap();
                let message: ToHelper = protocol::parse_message(&line).unwrap();
                if message != ToHelper::Heartbeat {
                    return message;
                }
            }
        }
    }

    fn connect(environment: Arc<FakeEnvironment>) -> (FakeHelper, std::thread::JoinHandle<anyhow::Result<()>>) {
        let (daemon_side, helper_side) = UnixStream::pair().unwrap();
        helper_side.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let session = std::thread::spawn(move || run_session(daemon_side, environment.as_ref(), &layouts()));
        let mut lines = BufReader::new(helper_side.try_clone().unwrap()).lines();
        let hello: ToHelper = protocol::parse_message(&lines.next().unwrap().unwrap()).unwrap();
        assert_eq!(hello, ToHelper::Hello { protocol: PROTOCOL_VERSION, role: Role::Session });
        (FakeHelper { stream: helper_side, lines }, session)
    }

    fn emit(usage: u16, pressed: bool) -> ToHelper {
        ToHelper::Emit { usage_page: PAGE_KEYBOARD, usage, pressed }
    }

    #[test]
    fn keys_come_back_remapped_and_seized_flips_the_strategy() {
        let environment = Arc::new(environment(true));
        let (mut helper, session) = connect(Arc::clone(&environment));
        helper.say(&ToDaemon::Welcome { protocol: PROTOCOL_VERSION });
        helper.say(&ToDaemon::Seized { active: true });
        helper.say(&ToDaemon::Key { usage_page: PAGE_KEYBOARD, usage: 0xE0, pressed: true, apple: false });
        helper.say(&ToDaemon::Key { usage_page: PAGE_KEYBOARD, usage: 0x06, pressed: true, apple: false });

        assert_eq!(helper.next_non_heartbeat(), emit(0xE0, true));
        assert_eq!(helper.next_non_heartbeat(), emit(0xE0, false));
        assert_eq!(helper.next_non_heartbeat(), emit(0xE3, true));
        assert_eq!(helper.next_non_heartbeat(), emit(0x06, true));
        assert_eq!(environment.input.strategy.get(), Strategy::VirtualHid);
        assert!(environment.input.markers.lookup(0x08, true, std::time::Instant::now()).is_some());

        helper.say(&ToDaemon::Seized { active: false });
        drop(helper);
        assert!(session.join().unwrap().is_ok());
        assert_eq!(environment.input.strategy.get(), Strategy::EventTap);
    }

    #[test]
    fn a_refused_greeting_names_both_versions() {
        let (mut helper, session) = connect(Arc::new(environment(true)));
        helper.say(&ToDaemon::Refused { protocol: 9, reason: "old".to_string() });
        let error = session.join().unwrap().unwrap_err().to_string();
        assert!(error.contains('9') && error.contains(&PROTOCOL_VERSION.to_string()), "{error}");
    }

    #[test]
    fn a_disabled_config_sends_no_heartbeats() {
        let (mut helper, _session) = connect(Arc::new(environment(false)));
        helper.say(&ToDaemon::Welcome { protocol: PROTOCOL_VERSION });
        helper.stream.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
        assert!(helper.lines.next().is_none_or(|line| line.is_err()));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p qol-keyremap virtual_hid::tests`
Expected: compile error, `cannot find function run_session`.

- [ ] **Step 3: Implement the session**

`virtual_hid/mod.rs`, above the tests:

```rust
pub(crate) mod fn_keys;
pub(crate) mod keyboard;

use std::collections::HashSet;
use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};

use crate::platform::macos::app::remap::ResolvedConfig;
use crate::platform::macos::hid_helper::protocol::{
    self, Role, ToDaemon, ToHelper, PAGE_KEYBOARD, PROTOCOL_VERSION, SOCKET_PATH,
};
use crate::platform::macos::input::{InputState, Strategy};
use crate::platform::macos::layout::LayoutStore;
use keyboard::{KeyContext, KeyboardState, Output};

const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(50);
const RETRY_INTERVAL: Duration = Duration::from_secs(2);
const CAPS_LOCK_SETTLE: Duration = Duration::from_millis(100);
const WRITE_TIMEOUT: Duration = Duration::from_millis(200);
const HID_SYSTEM_STATE: i32 = 1;
const ALPHA_SHIFT: u64 = 0x0001_0000;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFPreferencesAnyApplication: CFStringRef;
    fn CFPreferencesCopyAppValue(key: CFStringRef, application: CFStringRef) -> CFTypeRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceFlagsState(state: i32) -> u64;
}

pub(crate) trait KeyEnvironment: Send + Sync {
    fn config(&self) -> Arc<ResolvedConfig>;
    fn frontmost_bundle_id(&self) -> String;
    fn input(&self) -> &Arc<InputState>;
}

pub(crate) fn start(environment: Arc<impl KeyEnvironment + 'static>, layouts: Arc<LayoutStore>) {
    std::thread::Builder::new()
        .name("keyremap-vhid-session".into())
        .spawn(move || {
            let mut last_error = String::new();
            loop {
                let outcome = UnixStream::connect(SOCKET_PATH)
                    .context("the keyboard helper is not running")
                    .and_then(|stream| run_session(stream, environment.as_ref(), &layouts));
                match outcome {
                    Ok(()) => log::info!("the keyboard helper session ended"),
                    Err(error) => {
                        let message = format!("{error:#}");
                        if message != last_error {
                            log::info!("using the event tap: {message}");
                            last_error = message;
                        }
                    }
                }
                std::thread::sleep(RETRY_INTERVAL);
            }
        })
        .expect("failed to spawn the virtual HID session thread");
}

pub(crate) fn run_session(stream: UnixStream, environment: &impl KeyEnvironment, layouts: &LayoutStore) -> Result<()> {
    let result = session(stream, environment, layouts);
    if environment.input().strategy.set(Strategy::EventTap) == Strategy::VirtualHid {
        log::info!("key input is back on the event tap");
    }
    result
}

fn session(stream: UnixStream, environment: &impl KeyEnvironment, layouts: &LayoutStore) -> Result<()> {
    let mut writer = stream.try_clone()?;
    writer.set_write_timeout(Some(WRITE_TIMEOUT))?;
    protocol::write_message(&mut writer, &ToHelper::Hello { protocol: PROTOCOL_VERSION, role: Role::Session })?;
    let mut lines = BufReader::new(stream).lines();
    let greeting = lines.next().context("the keyboard helper closed the connection")??;
    match protocol::parse_message::<ToDaemon>(&greeting)? {
        ToDaemon::Welcome { .. } => log::info!("connected to the keyboard helper"),
        ToDaemon::Refused { protocol, reason } => bail!(
            "the keyboard helper speaks protocol {protocol} and keyremap speaks {PROTOCOL_VERSION} ({reason}); run sudo qol-keyremap install-hid-helper"
        ),
        other => bail!("unexpected greeting from the keyboard helper: {other:?}"),
    }

    let writer = Arc::new(Mutex::new(writer));
    let caps_due = Arc::new(Mutex::new(None::<Instant>));
    let stop = Arc::new(AtomicBool::new(false));
    std::thread::scope(|scope| {
        scope.spawn(|| heartbeat_loop(environment, &writer, &caps_due, &stop));
        let result = read_keys(lines, environment, layouts, &writer, &caps_due);
        stop.store(true, Ordering::SeqCst);
        result
    })
}

fn heartbeat_loop(
    environment: &impl KeyEnvironment,
    writer: &Mutex<UnixStream>,
    caps_due: &Mutex<Option<Instant>>,
    stop: &AtomicBool,
) {
    while !stop.load(Ordering::SeqCst) {
        if environment.config().enabled && send(writer, &ToHelper::Heartbeat).is_err() {
            return;
        }
        let due = caps_due
            .lock()
            .map(|mut due| due.take_if(|at| Instant::now() >= *at))
            .unwrap_or_default();
        if due.is_some() {
            let _ = send(writer, &ToHelper::CapsLockLight { on: caps_lock_on() });
        }
        std::thread::sleep(HEARTBEAT_INTERVAL);
    }
}

fn read_keys(
    lines: std::io::Lines<BufReader<UnixStream>>,
    environment: &impl KeyEnvironment,
    layouts: &LayoutStore,
    writer: &Mutex<UnixStream>,
    caps_due: &Mutex<Option<Instant>>,
) -> Result<()> {
    let mut keyboard = KeyboardState::default();
    let mut warned = HashSet::new();
    for line in lines {
        match protocol::parse_message::<ToDaemon>(&line?)? {
            ToDaemon::Key { usage_page, usage, pressed, apple } => {
                let config = environment.config();
                let layout = layouts.get();
                let bundle_id = environment.frontmost_bundle_id();
                let context = KeyContext {
                    config: &config,
                    table: &layout.table,
                    physical: layout.physical,
                    bundle_id: &bundle_id,
                    fn_state: apple && fn_keys_are_standard(),
                };
                for output in keyboard.handle(usage_page, usage, pressed, apple, &context) {
                    apply(output, environment.input(), writer, caps_due, &mut warned)?;
                }
            }
            ToDaemon::Seized { active } => {
                if !active {
                    keyboard.reset();
                }
                let strategy = if active { Strategy::VirtualHid } else { Strategy::EventTap };
                if environment.input().strategy.set(strategy) != strategy {
                    log::info!("key input strategy: {strategy:?}");
                }
            }
            ToDaemon::Welcome { .. } | ToDaemon::Refused { .. } | ToDaemon::Status(_) => {}
        }
    }
    Ok(())
}

fn apply(
    output: Output,
    input: &InputState,
    writer: &Mutex<UnixStream>,
    caps_due: &Mutex<Option<Instant>>,
    warned: &mut HashSet<String>,
) -> Result<()> {
    match output {
        Output::Emit(emit) => send(
            writer,
            &ToHelper::Emit { usage_page: emit.page, usage: emit.usage, pressed: emit.pressed },
        )?,
        Output::Mark { keycode, marker } => input.markers.insert(keycode, marker),
        Output::Unmark { keycode } => input.markers.release(keycode, Instant::now()),
        Output::MissingChar(text) => {
            if warned.insert(text.clone()) {
                log::warn!("no key sequence types {text:?} in the current keyboard layout; skipping it");
            }
        }
        Output::CapsLock => {
            if let Ok(mut due) = caps_due.lock() {
                *due = Some(Instant::now() + CAPS_LOCK_SETTLE);
            }
        }
    }
    Ok(())
}

fn send(writer: &Mutex<UnixStream>, message: &ToHelper) -> std::io::Result<()> {
    let mut stream = writer
        .lock()
        .map_err(|_| std::io::Error::other("helper writer lock poisoned"))?;
    protocol::write_message(&mut *stream, message)
}

fn fn_keys_are_standard() -> bool {
    let key = CFString::from_static_string("com.apple.keyboard.fnState");
    let value = unsafe { CFPreferencesCopyAppValue(key.as_concrete_TypeRef(), kCFPreferencesAnyApplication) };
    if value.is_null() {
        return false;
    }
    let value = unsafe { CFType::wrap_under_create_rule(value) };
    if let Some(flag) = value.downcast::<CFBoolean>() {
        return bool::from(flag);
    }
    value
        .downcast::<CFNumber>()
        .and_then(|number| number.to_i64())
        .is_some_and(|number| number != 0)
}

fn caps_lock_on() -> bool {
    unsafe { CGEventSourceFlagsState(HID_SYSTEM_STATE) } & ALPHA_SHIFT != 0
}
```

`Option::take_if` is stable since Rust 1.80.

In `tap.rs`:

```rust
impl super::input::backends::virtual_hid::KeyEnvironment for TapState {
    fn config(&self) -> Arc<ResolvedConfig> {
        TapState::config(self)
    }

    fn frontmost_bundle_id(&self) -> String {
        TapState::frontmost_bundle_id(self)
    }

    fn input(&self) -> &Arc<InputState> {
        TapState::input(self)
    }
}
```

- [ ] **Step 4: Wire it into the daemon and refresh the layout on the main thread**

`app/mod.rs`, full `run` and its imports:

```rust
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;
use std::time::Duration;

use super::input::backends::virtual_hid;
use super::input::InputState;
use super::layout::{LayoutSnapshot, LayoutStore};

const LAYOUT_POLL: Duration = Duration::from_secs(1);

pub(crate) fn run() {
    let raw_config = config::load_config();
    let resolved = remap::resolve(&raw_config);

    log::info!(
        "loaded {} char rules, {} key rules, {} mouse rules, {} scroll rules, {} excluded apps",
        resolved.char_rules.len(),
        resolved.key_rules.len(),
        resolved.mouse_rules.len(),
        resolved.scroll_rules.len(),
        resolved.excluded_apps.len(),
    );

    let layouts = Arc::new(LayoutStore::new(LayoutSnapshot::read_current().unwrap_or_else(
        |error| {
            log::warn!("no keyboard layout to type characters with: {error}");
            LayoutSnapshot::empty()
        },
    )));

    let (tx, rx) = std::sync::mpsc::channel();
    let Some((mut current_key_rules, state)) =
        start_services_if_singleton(daemon::start_listener(tx), || {
            let app_tracker = super::app_tracker::AppTracker::start();
            let current_key_rules = resolved.key_rules.clone();
            let state = Arc::new(super::tap::TapState::new(
                resolved,
                app_tracker,
                Arc::new(InputState::default()),
            ));
            super::tap::start_tap(Arc::clone(&state));
            virtual_hid::start(Arc::clone(&state), Arc::clone(&layouts));
            (current_key_rules, state)
        })
    else {
        if daemon::send_reload() {
            log::debug!("another instance running, sent reload");
        }
        return;
    };

    log::info!("daemon started");

    loop {
        let command = match rx.recv_timeout(LAYOUT_POLL) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => {
                layouts.refresh();
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        match command {
            daemon::Command::Reload => {
                let new_raw = config::load_config();
                let new_resolved = remap::resolve(&new_raw);
                log::debug!(
                    "reloaded {} char rules, {} key rules, {} mouse rules, {} scroll rules",
                    new_resolved.char_rules.len(),
                    new_resolved.key_rules.len(),
                    new_resolved.mouse_rules.len(),
                    new_resolved.scroll_rules.len(),
                );
                for warning in remap::diff_key_rules(&current_key_rules, &new_resolved.key_rules) {
                    log::warn!("{warning}");
                }
                current_key_rules = new_resolved.key_rules.clone();
                state.swap_config(new_resolved);
            }
            daemon::Command::Kill => {
                log::info!("kill received, shutting down");
                break;
            }
            daemon::Command::Settings => {
                if let Err(error) = qol_apps::desktop_integration::open_plugin_settings_via_tray(
                    crate::cli::PLUGIN_ID,
                ) {
                    log::warn!("failed to open settings page: {error}");
                }
            }
        }
        layouts.refresh();
    }

    daemon::cleanup();
}
```

`LayoutStore::refresh` is called only from this loop, which runs on the main thread, as TIS requires.

- [ ] **Step 5: Run the gate**

Run: `cargo test -p qol-keyremap && cargo clippy -p qol-keyremap --all-targets -- -D warnings`
Expected: PASS, including the three new session tests.

- [ ] **Step 6: Commit**

```bash
git add plugins/keyremap/src/platform/macos
git commit -m "feat(keyremap): remap keys through the keyboard helper when it is running"
```

---

### Task 13: Secure Input warning

**Files:**
- Create: `plugins/keyremap/src/platform/macos/secure_input/mod.rs`
- Create: `plugins/keyremap/src/platform/macos/secure_input/warning.rs`
- Modify: `plugins/keyremap/src/platform/mod.rs` (add `SecureInputHolder`)
- Modify: `plugins/keyremap/src/platform/macos/mod.rs` (add `mod secure_input;`)
- Modify: `plugins/keyremap/src/platform/macos/app/mod.rs` (start the watch)

**Interfaces:**
- Consumes: `Strategy`, `InputState` (Task 10).
- Produces:
  - `platform::SecureInputHolder { pid: i32, app: String }` (`Debug`, `Clone`, `PartialEq`, `Eq`)
  - `secure_input::holder() -> Option<SecureInputHolder>`
  - `secure_input::watch(input: Arc<InputState>)`
  - `warning::Toast { title: String, body: String }`, `warning::toast(&SecureInputHolder, Strategy) -> Toast`
  - `warning::SecureInputWatch::default()`, `observe(&mut self, holder: Option<&SecureInputHolder>, strategy: Strategy) -> Option<Toast>`

- [ ] **Step 1: Write the failing tests**

`warning.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn kitty() -> SecureInputHolder {
        SecureInputHolder { pid: 4242, app: "kitty".to_string() }
    }

    fn teams() -> SecureInputHolder {
        SecureInputHolder { pid: 777, app: "Microsoft Teams".to_string() }
    }

    #[test]
    fn warns_once_per_episode() {
        let mut watch = SecureInputWatch::default();
        assert!(watch.observe(Some(&kitty()), Strategy::EventTap).is_some());
        assert!(watch.observe(Some(&kitty()), Strategy::EventTap).is_none());
        assert!(watch.observe(Some(&kitty()), Strategy::VirtualHid).is_none());
    }

    #[test]
    fn a_new_holder_starts_a_new_episode() {
        let mut watch = SecureInputWatch::default();
        watch.observe(Some(&kitty()), Strategy::EventTap);
        let toast = watch.observe(Some(&teams()), Strategy::EventTap).unwrap();
        assert!(toast.title.contains("Microsoft Teams"));
    }

    #[test]
    fn secure_input_turning_off_ends_the_episode() {
        let mut watch = SecureInputWatch::default();
        watch.observe(Some(&kitty()), Strategy::EventTap);
        assert!(watch.observe(None, Strategy::EventTap).is_none());
        assert!(watch.observe(Some(&kitty()), Strategy::EventTap).is_some());
    }

    #[test]
    fn the_event_tap_text_says_key_remap_is_paused() {
        let toast = toast(&kitty(), Strategy::EventTap);
        assert_eq!(
            toast.title,
            "Key Remap and qol hotkeys are paused: kitty has turned on Secure Input."
        );
        assert!(toast.body.contains("Quit kitty"));
        assert!(toast.body.contains("virtual keyboard driver"));
    }

    #[test]
    fn the_virtual_hid_text_says_key_remap_keeps_working() {
        let toast = toast(&kitty(), Strategy::VirtualHid);
        assert_eq!(toast.title, "qol hotkeys are paused: kitty has turned on Secure Input.");
        assert!(toast.body.contains("Key Remap keeps working"));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p qol-keyremap secure_input`
Expected: compile error, `cannot find type SecureInputWatch`.

- [ ] **Step 3: Implement**

`platform/mod.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SecureInputHolder {
    pub(crate) pid: i32,
    pub(crate) app: String,
}
```

`secure_input/warning.rs`:

```rust
use crate::platform::macos::input::Strategy;
use crate::platform::SecureInputHolder;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Toast {
    pub(crate) title: String,
    pub(crate) body: String,
}

#[derive(Debug, Default)]
pub(crate) struct SecureInputWatch {
    warned_for: Option<i32>,
}

impl SecureInputWatch {
    pub(crate) fn observe(&mut self, holder: Option<&SecureInputHolder>, strategy: Strategy) -> Option<Toast> {
        let Some(holder) = holder else {
            self.warned_for = None;
            return None;
        };
        if self.warned_for == Some(holder.pid) {
            return None;
        }
        self.warned_for = Some(holder.pid);
        Some(toast(holder, strategy))
    }
}

pub(crate) fn toast(holder: &SecureInputHolder, strategy: Strategy) -> Toast {
    let app = &holder.app;
    match strategy {
        Strategy::EventTap => Toast {
            title: format!("Key Remap and qol hotkeys are paused: {app} has turned on Secure Input."),
            body: format!(
                "Quit {app} or turn off its secure keyboard entry to get them back. \
Installing the virtual keyboard driver keeps Key Remap working through this; \
qol-keyremap doctor shows the steps."
            ),
        },
        Strategy::VirtualHid => Toast {
            title: format!("qol hotkeys are paused: {app} has turned on Secure Input."),
            body: format!(
                "Key Remap keeps working. Quit {app} or turn off its secure keyboard entry to get qol hotkeys back."
            ),
        },
    }
}
```

`secure_input/mod.rs`:

```rust
pub(crate) mod warning;

use std::sync::Arc;
use std::time::Duration;

use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use objc2::rc::autoreleasepool;
use objc2_app_kit::NSRunningApplication;
use qol_runtime::protocol::NotificationLevel;

use crate::platform::macos::input::InputState;
use crate::platform::SecureInputHolder;
use warning::SecureInputWatch;

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const UNKNOWN_APP: &str = "An app";

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn IsSecureEventInputEnabled() -> u8;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGSessionCopyCurrentDictionary() -> CFDictionaryRef;
}

pub(crate) fn holder() -> Option<SecureInputHolder> {
    if unsafe { IsSecureEventInputEnabled() } == 0 {
        return None;
    }
    let pid = holder_pid();
    Some(SecureInputHolder {
        pid: pid.unwrap_or(0),
        app: pid.and_then(app_name).unwrap_or_else(|| UNKNOWN_APP.to_string()),
    })
}

pub(crate) fn watch(input: Arc<InputState>) {
    std::thread::Builder::new()
        .name("keyremap-secure-input".into())
        .spawn(move || {
            let mut watch = SecureInputWatch::default();
            let client = qol_runtime::PlatformStateClient::from_env();
            loop {
                if let Some(toast) = watch.observe(holder().as_ref(), input.strategy.get()) {
                    log::warn!("{}", toast.title);
                    if !client.send_notification(&toast.title, &toast.body, NotificationLevel::Warn) {
                        log::warn!("could not show the Secure Input warning in qol-tray");
                    }
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        })
        .expect("failed to spawn the Secure Input watch thread");
}

fn holder_pid() -> Option<i32> {
    let dictionary = unsafe { CGSessionCopyCurrentDictionary() };
    if dictionary.is_null() {
        return None;
    }
    let dictionary: CFDictionary<CFString, CFType> = unsafe { CFDictionary::wrap_under_create_rule(dictionary) };
    let key = CFString::from_static_string("kCGSSessionSecureInputPID");
    dictionary.find(&key)?.downcast::<CFNumber>()?.to_i32()
}

fn app_name(pid: i32) -> Option<String> {
    autoreleasepool(|_| {
        NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
            .and_then(|app| app.localizedName())
            .map(|name| name.to_string())
    })
}
```

If `runningApplicationWithProcessIdentifier` needs an `unsafe` block in this objc2 version, wrap it the same way `app_tracker.rs` does.

`app/mod.rs`, inside the singleton closure after `virtual_hid::start(...)`:

```rust
            super::secure_input::watch(Arc::clone(state.input()));
```

- [ ] **Step 4: Run the gate**

Run: `cargo test -p qol-keyremap && cargo clippy -p qol-keyremap --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add plugins/keyremap/src/platform
git commit -m "feat(keyremap): warn when an app turns on Secure Input"
```

---

### Task 14: Doctor checks

**Files:**
- Create: `plugins/keyremap/src/platform/macos/virtual_hid/driver.rs`
- Modify: `plugins/keyremap/src/platform/macos/virtual_hid/mod.rs` (add `pub(crate) mod driver;`)
- Modify: `plugins/keyremap/src/platform/macos/hid_helper/mod.rs` (add `query_status`)
- Modify: `plugins/keyremap/src/platform/macos/app/remap.rs` (add `character_targets`)
- Modify: `plugins/keyremap/src/platform/mod.rs`, the four adapters, `plugins/keyremap/src/platform/macos/mod.rs`, `plugins/keyremap/src/platform/macos/input/backends/mod.rs` (drop every `#[allow(dead_code)]` this plan added)
- Modify: `plugins/keyremap/src/cli.rs`

**Interfaces:**
- Consumes: everything above, plus `qol_plugin_api::manifest::{PluginManifest, SystemDependency, parse_version}` (Task 1).
- Produces, in `platform/mod.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Probe<T> {
    Known(T),
    Unknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExtensionState {
    Activated,
    NotActivated,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DriverState {
    pub(crate) installed_version: Option<String>,
    pub(crate) extension: ExtensionState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HelperReport {
    pub(crate) virtual_keyboard_ready: bool,
    pub(crate) input_monitoring: bool,
    pub(crate) seized: Vec<String>,
    pub(crate) conflicts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HelperState {
    NotInstalled,
    NotRunning,
    VersionMismatch { helper: u32, expected: u32 },
    Running(HelperReport),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LayoutGap {
    pub(crate) rule: String,
    pub(crate) character: String,
}
```

and on the trait:

```rust
    fn virtual_hid_driver(&self) -> Probe<DriverState>;
    fn virtual_hid_daemon(&self) -> Probe<bool>;
    fn hid_helper_state(&self) -> Probe<HelperState>;
    fn secure_input(&self) -> Probe<Option<SecureInputHolder>>;
    fn layout_gaps(&self) -> Probe<Vec<LayoutGap>>;
```

All five virtual HID checks report `Warn`, never `Fail`: the spec treats a missing driver as a warning because the event tap still works, and the daemon, helper and layout checks describe the same optional path. A probe that could not answer is `Warn` naming what could not be read, never `Ok`.

- [ ] **Step 1: Write the failing tests**

`driver.rs` tests, with fixtures copied from this Mac on 2026-09-30:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const PKGUTIL: &str = "package-id: org.pqrs.Karabiner-DriverKit-VirtualHIDDevice\nversion: 8.6.0\nvolume: /\nlocation: \ninstall-time: 1790761368\n";
    const EXTENSIONS: &str = "3 extension(s)\n--- com.apple.system_extension.driver_extension\nenabled\tactive\tteamID\tbundleID (version)\tname\t[state]\n*\t*\tG43BCU2T37\torg.pqrs.Karabiner-DriverKit-VirtualHIDDevice (1.8.0/1.8.0)\torg.pqrs.Karabiner-DriverKit-VirtualHIDDevice\t[activated enabled]\n";

    #[test]
    fn the_package_version_is_read_from_pkgutil() {
        assert_eq!(parse_pkg_version(PKGUTIL), Some("8.6.0".to_string()));
        assert_eq!(parse_pkg_version("No receipt for 'x' found at '/'."), None);
    }

    #[test]
    fn the_extension_state_is_read_from_systemextensionsctl() {
        assert_eq!(parse_extension_state(EXTENSIONS), ExtensionState::Activated);
        assert_eq!(
            parse_extension_state(&EXTENSIONS.replace("[activated enabled]", "[activated waiting for user]")),
            ExtensionState::NotActivated
        );
        assert_eq!(parse_extension_state("0 extension(s)\n"), ExtensionState::Missing);
    }

    #[test]
    fn a_running_launchd_job_says_so() {
        assert!(parse_launchd_running("system/x = {\n\tstate = running\n\tpid = 10\n}"));
        assert!(!parse_launchd_running("system/x = {\n\tstate = not running\n}"));
    }
}
```

`remap.rs`, add to its test module:

```rust
    #[test]
    fn character_targets_name_every_rule_that_types_text() {
        let raw: RemapConfig = serde_json::from_value(serde_json::json!({
            "char_swaps": [["$", "€"]],
            "char_rules": [{ "from_mods": ["ralt"], "from_key": "2", "to_char": "@" }],
            "key_rules": [
                { "from_key": "f13", "to_key": "~" },
                { "from_mods": ["ctrl"], "from_key": "c", "to_mods": ["cmd"], "to_key": "c" }
            ]
        }))
        .unwrap();
        let targets = character_targets(&resolve(&raw));
        let texts: Vec<&str> = targets.iter().map(|(_, text)| text.as_str()).collect();
        assert_eq!(texts, vec!["@", "€", "~"]);
        assert!(targets[0].0.contains("ralt+2"), "{}", targets[0].0);
    }
```

`cli.rs` tests. Extend `SentinelAdapter` with healthy probes:

```rust
        fn virtual_hid_driver(&self) -> Probe<DriverState> {
            Probe::Known(DriverState {
                installed_version: Some("8.6.0".to_string()),
                extension: ExtensionState::Activated,
            })
        }

        fn virtual_hid_daemon(&self) -> Probe<bool> {
            Probe::Known(true)
        }

        fn hid_helper_state(&self) -> Probe<HelperState> {
            Probe::Known(HelperState::Running(HelperReport {
                virtual_keyboard_ready: true,
                input_monitoring: true,
                seized: vec!["Apple Internal Keyboard / Trackpad".to_string()],
                conflicts: Vec::new(),
            }))
        }

        fn secure_input(&self) -> Probe<Option<SecureInputHolder>> {
            Probe::Known(None)
        }

        fn layout_gaps(&self) -> Probe<Vec<LayoutGap>> {
            Probe::Known(Vec::new())
        }
```

change `assert_eq!(report.checks.len(), 4);` to `9`, and add:

```rust
    fn driver() -> SystemDependency {
        required_driver().unwrap()
    }

    #[test]
    fn the_driver_minimum_comes_from_the_manifest() {
        assert_eq!(driver().min_version, "8.6.0");
    }

    #[test]
    fn a_missing_or_old_driver_warns_with_the_download_url() {
        let missing = driver_result(
            &Probe::Known(DriverState { installed_version: None, extension: ExtensionState::Missing }),
            &driver(),
        );
        assert_eq!(missing.status, DoctorStatus::Warn);
        assert!(missing.fix.unwrap().contains("github.com/pqrs-org"));

        let old = driver_result(
            &Probe::Known(DriverState {
                installed_version: Some("8.5.9".to_string()),
                extension: ExtensionState::Activated,
            }),
            &driver(),
        );
        assert_eq!(old.status, DoctorStatus::Warn);
        assert!(old.message.contains("8.5.9"));
    }

    #[test]
    fn an_inactive_extension_names_install_hid_helper() {
        let result = driver_result(
            &Probe::Known(DriverState {
                installed_version: Some("8.6.0".to_string()),
                extension: ExtensionState::NotActivated,
            }),
            &driver(),
        );
        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.fix.unwrap().contains("install-hid-helper"));
    }

    #[test]
    fn helper_states_map_to_warnings_with_the_install_fix() {
        for state in [
            HelperState::NotInstalled,
            HelperState::NotRunning,
            HelperState::VersionMismatch { helper: 0, expected: 1 },
        ] {
            let result = helper_result(&Probe::Known(state));
            assert_eq!(result.status, DoctorStatus::Warn);
            assert!(result.fix.unwrap().contains("install-hid-helper"));
        }
    }

    #[test]
    fn a_helper_without_input_monitoring_says_how_to_turn_it_on() {
        let result = helper_result(&Probe::Known(HelperState::Running(HelperReport {
            virtual_keyboard_ready: true,
            input_monitoring: false,
            seized: Vec::new(),
            conflicts: Vec::new(),
        })));
        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.fix.unwrap().contains("Input Monitoring"));
    }

    #[test]
    fn a_keyboard_held_by_another_app_is_named() {
        let result = helper_result(&Probe::Known(HelperState::Running(HelperReport {
            virtual_keyboard_ready: true,
            input_monitoring: true,
            seized: Vec::new(),
            conflicts: vec!["Keychron K2".to_string()],
        })));
        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.message.contains("Keychron K2"));
    }

    #[test]
    fn secure_input_names_the_app_and_the_strategy() {
        let helper = Probe::Known(HelperState::NotRunning);
        let result = secure_input_result(
            &Probe::Known(Some(SecureInputHolder { pid: 4242, app: "kitty".to_string() })),
            &helper,
        );
        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.message.contains("kitty") && result.message.contains("4242"));
        assert!(result.message.contains("event_tap"));

        let quiet = secure_input_result(&Probe::Known(None), &helper);
        assert_eq!(quiet.status, DoctorStatus::Ok);
    }

    #[test]
    fn untypeable_characters_name_their_rule() {
        let result = layout_result(&Probe::Known(vec![LayoutGap {
            rule: "char rule ralt+3 -> あ".to_string(),
            character: "あ".to_string(),
        }]));
        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.message.contains("char rule ralt+3"));
    }

    #[test]
    fn unknown_probes_warn_instead_of_passing() {
        let unknown = || Probe::Unknown("pkgutil is missing".to_string());
        assert_eq!(driver_result(&unknown(), &driver()).status, DoctorStatus::Warn);
        assert_eq!(daemon_result(&unknown()).status, DoctorStatus::Warn);
        assert_eq!(helper_result(&unknown()).status, DoctorStatus::Warn);
        assert_eq!(layout_result(&unknown()).status, DoctorStatus::Warn);
    }
```

Import `DoctorStatus` from `qol_headless` in the test module, and `SystemDependency` from `qol_plugin_api::manifest`. `DoctorCheckResult` exposes `status`, `message` and `fix: Option<String>` (see `libs/headless/src/doctor/contract.rs:179`).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p qol-keyremap`
Expected: compile errors for the missing probes and result functions.

- [ ] **Step 3: Implement the probes**

`virtual_hid/driver.rs`:

```rust
use std::process::Command;

use super::{OWN_DAEMON_LABEL, PQRS_DAEMON_LABEL};
use crate::platform::{DriverState, ExtensionState, Probe};

const PACKAGE_ID: &str = "org.pqrs.Karabiner-DriverKit-VirtualHIDDevice";

pub(crate) fn driver_state() -> Probe<DriverState> {
    let installed_version = match Command::new("/usr/sbin/pkgutil").args(["--pkg-info", PACKAGE_ID]).output() {
        Ok(output) if output.status.success() => parse_pkg_version(&String::from_utf8_lossy(&output.stdout)),
        Ok(_) => None,
        Err(error) => return Probe::Unknown(format!("could not run pkgutil: {error}")),
    };
    let extension = match Command::new("/usr/bin/systemextensionsctl").arg("list").output() {
        Ok(output) => parse_extension_state(&String::from_utf8_lossy(&output.stdout)),
        Err(error) => return Probe::Unknown(format!("could not run systemextensionsctl: {error}")),
    };
    Probe::Known(DriverState { installed_version, extension })
}

pub(crate) fn daemon_running() -> Probe<bool> {
    for label in [PQRS_DAEMON_LABEL, OWN_DAEMON_LABEL] {
        match Command::new("/bin/launchctl").args(["print", &format!("system/{label}")]).output() {
            Ok(output) if output.status.success() => {
                if parse_launchd_running(&String::from_utf8_lossy(&output.stdout)) {
                    return Probe::Known(true);
                }
            }
            Ok(_) => {}
            Err(error) => return Probe::Unknown(format!("could not run launchctl: {error}")),
        }
    }
    Probe::Known(false)
}

fn parse_pkg_version(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("version: "))
        .map(|version| version.trim().to_string())
}

fn parse_extension_state(output: &str) -> ExtensionState {
    let mut lines = output.lines().filter(|line| line.contains(PACKAGE_ID)).peekable();
    if lines.peek().is_none() {
        return ExtensionState::Missing;
    }
    if lines.any(|line| line.contains("[activated enabled]")) {
        ExtensionState::Activated
    } else {
        ExtensionState::NotActivated
    }
}

fn parse_launchd_running(output: &str) -> bool {
    output.lines().any(|line| line.trim() == "state = running")
}
```

`hid_helper/mod.rs`, add:

```rust
pub(crate) fn query_status() -> crate::platform::Probe<crate::platform::HelperState> {
    use std::io::{BufRead, BufReader, ErrorKind};

    use crate::platform::{HelperReport, HelperState, Probe};

    let stream = match UnixStream::connect(protocol::SOCKET_PATH) {
        Ok(stream) => stream,
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) => {
            return Probe::Known(if Path::new(install::HELPER_PLIST).exists() {
                HelperState::NotRunning
            } else {
                HelperState::NotInstalled
            });
        }
        Err(error) if error.kind() == ErrorKind::PermissionDenied => {
            return Probe::Unknown("the helper socket belongs to another user".to_string());
        }
        Err(error) => return Probe::Unknown(format!("could not reach the helper: {error}")),
    };
    let exchange = || -> anyhow::Result<ToDaemon> {
        stream.set_read_timeout(Some(Duration::from_secs(1)))?;
        let mut writer = stream.try_clone()?;
        protocol::write_message(
            &mut writer,
            &protocol::ToHelper::Hello { protocol: PROTOCOL_VERSION, role: protocol::Role::Status },
        )?;
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line)?;
        Ok(protocol::parse_message(&line)?)
    };
    match exchange() {
        Ok(ToDaemon::Status(status)) => Probe::Known(HelperState::Running(HelperReport {
            virtual_keyboard_ready: status.virtual_keyboard_ready,
            input_monitoring: status.input_monitoring,
            seized: status.seized,
            conflicts: status.conflicts,
        })),
        Ok(ToDaemon::Refused { protocol, .. }) => Probe::Known(HelperState::VersionMismatch {
            helper: protocol,
            expected: PROTOCOL_VERSION,
        }),
        Ok(other) => Probe::Unknown(format!("the helper answered {other:?}")),
        Err(error) => Probe::Unknown(format!("the helper did not answer: {error:#}")),
    }
}
```

`app/remap.rs`, add next to `diff_key_rules`:

```rust
pub fn character_targets(config: &ResolvedConfig) -> Vec<(String, String)> {
    let mut targets = Vec::new();
    for rule in &config.char_rules {
        targets.push((
            format!("char rule {} -> {}", rule_label(&rule.from_mods, rule.from_key), rule.to_char),
            rule.to_char.clone(),
        ));
    }
    for rule in &config.char_swap_rules {
        targets.push((format!("char swap {} -> {}", rule.from_char, rule.to_char), rule.to_char.clone()));
    }
    for rule in &config.key_rules {
        if let ResolvedKeyTarget::Char { text } = &rule.to {
            targets.push((
                format!("key rule {} -> {text}", rule_label(&rule.from_mods, rule.from_key)),
                text.clone(),
            ));
        }
    }
    targets
}
```

macOS adapter, in `platform/macos/mod.rs` (extend the `use super::{...}` line with `DriverState, HelperState, LayoutGap, Probe, SecureInputHolder`):

```rust
    fn virtual_hid_driver(&self) -> Probe<DriverState> {
        virtual_hid::driver::driver_state()
    }

    fn virtual_hid_daemon(&self) -> Probe<bool> {
        virtual_hid::driver::daemon_running()
    }

    fn hid_helper_state(&self) -> Probe<HelperState> {
        hid_helper::query_status()
    }

    fn secure_input(&self) -> Probe<Option<SecureInputHolder>> {
        Probe::Known(secure_input::holder())
    }

    fn layout_gaps(&self) -> Probe<Vec<LayoutGap>> {
        let snapshot = match layout::LayoutSnapshot::read_current() {
            Ok(snapshot) => snapshot,
            Err(error) => return Probe::Unknown(error.to_string()),
        };
        let resolved = app::remap::resolve(&app::config::load_config());
        let gaps = app::remap::character_targets(&resolved)
            .into_iter()
            .flat_map(|(rule, text)| {
                layout::missing_characters(&snapshot.table, [text.as_str()])
                    .into_iter()
                    .map(move |character| LayoutGap { rule: rule.clone(), character })
            })
            .collect();
        Probe::Known(gaps)
    }
```

Linux, Windows and fallback adapters:

```rust
    fn virtual_hid_driver(&self) -> Probe<DriverState> {
        Probe::Unknown(MACOS_ONLY.to_string())
    }

    fn virtual_hid_daemon(&self) -> Probe<bool> {
        Probe::Unknown(MACOS_ONLY.to_string())
    }

    fn hid_helper_state(&self) -> Probe<HelperState> {
        Probe::Unknown(MACOS_ONLY.to_string())
    }

    fn secure_input(&self) -> Probe<Option<SecureInputHolder>> {
        Probe::Unknown(MACOS_ONLY.to_string())
    }

    fn layout_gaps(&self) -> Probe<Vec<LayoutGap>> {
        Probe::Unknown(MACOS_ONLY.to_string())
    }
```

with `pub(crate) const MACOS_ONLY: &str = "the virtual keyboard path only exists on macOS";` defined once in `platform/mod.rs`.

- [ ] **Step 4: Implement the checks**

`cli.rs`:

```rust
use qol_plugin_api::manifest::{parse_version, PluginManifest, SystemDependency};

use crate::platform::{
    DriverState, ExtensionState, HelperState, LayoutGap, Probe, SecureInputHolder,
};

const MANIFEST: &str = include_str!("../plugin.toml");
const DRIVER_NAME: &str = "Karabiner-DriverKit-VirtualHIDDevice";

fn required_driver() -> Result<SystemDependency> {
    PluginManifest::parse_and_validate(MANIFEST)?
        .dependencies
        .and_then(|dependencies| dependencies.system.into_iter().find(|system| system.name == DRIVER_NAME))
        .ok_or_else(|| anyhow::anyhow!("plugin.toml does not declare {DRIVER_NAME}"))
}

fn install_command() -> String {
    let binary = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| PLUGIN_ID.to_string());
    format!("sudo \"{binary}\" install-hid-helper")
}

fn install_fix() -> String {
    format!("Run: {}", install_command())
}
```

In `doctor_checks`, replace `let trust = adapter;` with:

```rust
    let trust = adapter.clone();
    let driver = adapter.clone();
    let daemon = adapter.clone();
    let helper = adapter.clone();
    let secure = adapter.clone();
    let characters = adapter;
```

and append to the `vec![...]`:

```rust
        DoctorCheck::new(
            "virtual_hid_driver",
            "Check the virtual keyboard driver package and its system extension.",
            move || {
                supported_or(&driver, "virtual_hid_driver", |adapter| {
                    Ok(driver_result(&adapter.virtual_hid_driver(), &required_driver()?))
                })
            },
        ),
        DoctorCheck::new(
            "virtual_hid_daemon",
            "Check the virtual keyboard daemon is running.",
            move || {
                supported_or(&daemon, "virtual_hid_daemon", |adapter| {
                    Ok(daemon_result(&adapter.virtual_hid_daemon()))
                })
            },
        ),
        DoctorCheck::new(
            "hid_helper",
            "Ask the root keyboard helper for its status without changing anything.",
            move || {
                supported_or(&helper, "hid_helper", |adapter| {
                    Ok(helper_result(&adapter.hid_helper_state()))
                })
            },
        ),
        DoctorCheck::new(
            "secure_input",
            "Report which app holds Secure Input and which key strategy is active.",
            move || {
                supported_or(&secure, "secure_input", |adapter| {
                    Ok(secure_input_result(&adapter.secure_input(), &adapter.hid_helper_state()))
                })
            },
        ),
        DoctorCheck::new(
            "layout_characters",
            "Check every character rule can be typed in the active keyboard layout.",
            move || {
                supported_or(&characters, "layout_characters", |adapter| {
                    Ok(layout_result(&adapter.layout_gaps()))
                })
            },
        ),
```

and the result functions:

```rust
fn supported_or<A: PlatformAdapter>(
    adapter: &A,
    id: &str,
    check: impl FnOnce(&A) -> Result<DoctorCheckResult>,
) -> Result<DoctorCheckResult> {
    if !adapter.supported() {
        return Ok(DoctorCheckResult::fail(id, "The virtual keyboard path is unavailable on this platform.")
            .with_fix("Run Key Remap on macOS."));
    }
    check(adapter)
}

fn unknown(id: &str, reason: &str) -> DoctorCheckResult {
    DoctorCheckResult::warn(id, format!("Could not tell: {reason}."))
}

fn driver_result(probe: &Probe<DriverState>, required: &SystemDependency) -> DoctorCheckResult {
    let id = "virtual_hid_driver";
    let state = match probe {
        Probe::Known(state) => state,
        Probe::Unknown(reason) => return unknown(id, reason),
    };
    let download = format!(
        "Install {} {} or newer from {}, then run: {}",
        required.name,
        required.min_version,
        required.url,
        install_command()
    );
    let Some(installed) = &state.installed_version else {
        return DoctorCheckResult::warn(
            id,
            "The virtual keyboard driver is not installed, so Key Remap stops while an app holds Secure Input.",
        )
        .with_fix(download);
    };
    let current_enough = match (parse_version(installed), parse_version(&required.min_version)) {
        (Some(installed), Some(minimum)) => installed >= minimum,
        _ => false,
    };
    if !current_enough {
        return DoctorCheckResult::warn(
            id,
            format!("The virtual keyboard driver is {installed}, older than {}.", required.min_version),
        )
        .with_fix(download);
    }
    match state.extension {
        ExtensionState::Activated => DoctorCheckResult::ok(
            id,
            format!("The virtual keyboard driver {installed} is installed and its extension is active."),
        ),
        ExtensionState::NotActivated | ExtensionState::Missing => DoctorCheckResult::warn(
            id,
            format!("The virtual keyboard driver {installed} is installed but its extension is not active."),
        )
        .with_fix(install_fix()),
    }
}

fn daemon_result(probe: &Probe<bool>) -> DoctorCheckResult {
    let id = "virtual_hid_daemon";
    match probe {
        Probe::Known(true) => DoctorCheckResult::ok(id, "The virtual keyboard daemon is running."),
        Probe::Known(false) => DoctorCheckResult::warn(id, "The virtual keyboard daemon is not running.")
            .with_fix(install_fix()),
        Probe::Unknown(reason) => unknown(id, reason),
    }
}

fn helper_result(probe: &Probe<HelperState>) -> DoctorCheckResult {
    let id = "hid_helper";
    let report = match probe {
        Probe::Unknown(reason) => return unknown(id, reason),
        Probe::Known(HelperState::NotInstalled) => {
            return DoctorCheckResult::warn(id, "The keyboard helper is not installed, so Key Remap uses the event tap.")
                .with_fix(install_fix());
        }
        Probe::Known(HelperState::NotRunning) => {
            return DoctorCheckResult::warn(id, "The keyboard helper is installed but launchd has not started it.")
                .with_fix(install_fix());
        }
        Probe::Known(HelperState::VersionMismatch { helper, expected }) => {
            return DoctorCheckResult::warn(
                id,
                format!("The keyboard helper speaks protocol {helper} and this Key Remap speaks {expected}."),
            )
            .with_fix(install_fix());
        }
        Probe::Known(HelperState::Running(report)) => report,
    };
    if !report.input_monitoring {
        return DoctorCheckResult::warn(id, "The keyboard helper cannot read the keyboard: Input Monitoring is off.")
            .with_fix("Turn on com.qol-tools.keyremap.hid-helper in System Settings > Privacy & Security > Input Monitoring.");
    }
    if !report.conflicts.is_empty() {
        return DoctorCheckResult::warn(
            id,
            format!("Another app holds these keyboards: {}.", report.conflicts.join(", ")),
        )
        .with_fix("Quit the app that grabs the keyboard, such as Karabiner-Elements.");
    }
    if !report.virtual_keyboard_ready {
        return DoctorCheckResult::warn(id, "The keyboard helper is running but the virtual keyboard is not ready.")
            .with_fix(install_fix());
    }
    let seized = if report.seized.is_empty() {
        "no keyboards right now, because Key Remap is not connected".to_string()
    } else {
        report.seized.join(", ")
    };
    DoctorCheckResult::ok(id, format!("The keyboard helper is running and holds {seized}."))
}

fn secure_input_result(holder: &Probe<Option<SecureInputHolder>>, helper: &Probe<HelperState>) -> DoctorCheckResult {
    let id = "secure_input";
    let virtual_hid = matches!(
        helper,
        Probe::Known(HelperState::Running(report)) if !report.seized.is_empty()
    );
    let strategy = if virtual_hid { "virtual_hid" } else { "event_tap" };
    match holder {
        Probe::Unknown(reason) => unknown(id, reason),
        Probe::Known(None) => DoctorCheckResult::ok(
            id,
            format!("No app holds Secure Input. Key input strategy: {strategy}."),
        ),
        Probe::Known(Some(holder)) => {
            let paused = if virtual_hid { "qol hotkeys are paused" } else { "Key Remap and qol hotkeys are paused" };
            DoctorCheckResult::warn(
                id,
                format!(
                    "{} (pid {}) holds Secure Input, so {paused}. Key input strategy: {strategy}.",
                    holder.app, holder.pid
                ),
            )
            .with_fix(format!("Quit {} or turn off its secure keyboard entry.", holder.app))
        }
    }
}

fn layout_result(probe: &Probe<Vec<LayoutGap>>) -> DoctorCheckResult {
    let id = "layout_characters";
    match probe {
        Probe::Unknown(reason) => unknown(id, reason),
        Probe::Known(gaps) if gaps.is_empty() => {
            DoctorCheckResult::ok(id, "Every character rule can be typed in the active keyboard layout.")
        }
        Probe::Known(gaps) => {
            let listed: Vec<String> = gaps
                .iter()
                .map(|gap| format!("{:?} ({})", gap.character, gap.rule))
                .collect();
            DoctorCheckResult::warn(
                id,
                format!("The active keyboard layout cannot type {}.", listed.join(", ")),
            )
            .with_fix("Switch to a layout that has these characters, or change the rules.")
        }
    }
}
```


Remove every `#[allow(dead_code)]` this plan added (`layout`, `virtual_hid`, `hid_helper`, `input` in `platform/macos/mod.rs`, and `virtual_hid` in `input/backends/mod.rs`). `git diff main~14 -- plugins/keyremap/src | grep -c 'allow(dead_code)'` counts only removals afterwards.

- [ ] **Step 5: Run the gate**

Run: `cargo test -p qol-keyremap && cargo clippy -p qol-keyremap --all-targets -- -D warnings && cargo run -p qol-keyremap -- doctor`, then the Linux compile check from Global Constraints.
Expected: tests and clippy PASS, and doctor prints nine checks. On this Mac before Task 15, `hid_helper` warns "not installed" and `virtual_hid_daemon` warns "not running".

- [ ] **Step 6: Commit**

```bash
git add plugins/keyremap/src
git commit -m "feat(keyremap): report the virtual keyboard path in doctor"
```

---

### Task 15: End to end on this Mac

**Files:** none changed unless a check fails; a failure becomes a fix in the task that owns the code, with a regression test there first.

- [ ] **Step 1: Install**

```bash
cargo build -p qol-keyremap --release
sudo ./target/release/qol-keyremap install-hid-helper
```

Expected: the step lines, ending with the Input Monitoring note. If macOS shows the Input Monitoring prompt, turn on `com.qol-tools.keyremap.hid-helper`. Then `./target/release/qol-keyremap doctor` shows `virtual_hid_driver`, `virtual_hid_daemon` and `hid_helper` as ok.

- [ ] **Step 2: Run the daemon through qol**

Run `qol dev` and wait for the keyremap log line `key input strategy: VirtualHid`. `qol-keyremap doctor` now says `hid_helper` holds the built-in keyboard and `secure_input` says `virtual_hid`.

- [ ] **Step 3: Repeat the 2026-09-30 probe through the real code**

Turn on kitty's Secure Keyboard Entry and focus kitty. Check each:

1. Ctrl+C copies, as Cmd+C.
2. Right Option+2 types `@`; Left Option+2 does not.
3. Right Option+] types `~`.
4. The key left of 1 and the key left of Z type what their caps say (ISO keyboard).
5. F1, F2 change brightness; F10, F11, F12 mute and change volume; F3 opens Mission Control; with Fn held they are F-keys.
6. Fn+Left goes to line start; Fn+Backspace deletes forward.
7. Caps Lock toggles and its light follows.
8. A toast says "qol hotkeys are paused: kitty has turned on Secure Input." and says Key Remap keeps working.

Turn Secure Keyboard Entry off, then check a qol hotkey that uses Ctrl while a Ctrl rule is active still fires the qol hotkey (the marker path).

- [ ] **Step 4: The keyboard must never die**

Run `pkill -9 -f 'qol-keyremap$'` or kill the keyremap daemon from the qol console, then type immediately.
Expected: typing works again within 200 ms, unremapped. `sudo tail /var/log/com.qol-tools.keyremap.hid-helper.log` shows `released <keyboard>`.

- [ ] **Step 5: Hot-plug**

With the daemon running, plug in a USB keyboard, type Ctrl+C on it, then unplug it while holding Ctrl.
Expected: the new keyboard is remapped, and after unplugging no modifier stays stuck (typing `a` on the built-in keyboard gives `a`).

- [ ] **Step 6: The fallback warning**

Run `sudo launchctl bootout system/com.qol-tools.keyremap.hid-helper`, turn Secure Keyboard Entry on in kitty.
Expected: the toast says "Key Remap and qol hotkeys are paused: kitty has turned on Secure Input." Outside kitty, remapping works through the event tap as it does today. Restore with `sudo ./target/release/qol-keyremap install-hid-helper`.

- [ ] **Step 7: Record the result**

Update the memory file `macos-secure-input-kills-taps.md` with what shipped and which checks passed, and commit any fixes found on the way (each with its test).

