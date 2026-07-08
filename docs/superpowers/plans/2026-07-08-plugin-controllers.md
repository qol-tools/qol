# plugin-controllers Implementation Plan

> **For agentic workers:** This plan is executed inline in the authoring session per user instruction (no subagents). Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Linux qol-tray plugin that detects known-broken game controllers and applies driver fixes (first entry: GuliKit Controller XW xpadneo quirk 263) with one explicit authorization per PC.

**Architecture:** Pure modules (fix database, /proc/bus/input/devices parser, fix-state computation, config composition) with all I/O at the edges: a socket daemon that polls for controllers and notifies, a pkexec apply action, and a HeadlessApp CLI with doctor checks. Follows plugin-lights' never-escalate daemon pattern and qol-shot's CLI/daemon dispatch.

**Tech Stack:** Rust, qol-plugin-daemon (socket + notifications), qol-headless (CLI + doctor), qol-conventions. No new external crates.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-07-08-plugin-controllers-design.md`.
- `platforms = ["linux"]` in plugin.toml; the crate must still compile on macOS/Windows (workspace glob), so no unix-only APIs outside what qol-plugin-daemon already gates.
- No code comments. Table-driven tests with context in assertions. Conventional one-line commits, no attribution.
- The daemon never writes system state and never prompts; privileged writes happen only inside the user-triggered `apply_fixes` action via pkexec.
- Fix persisted-state detection must scan ALL `/etc/modprobe.d/*.conf`, so a hand-configured PC (like this one) reads as `Applied` and never gets duplicated config.
- Daemon port 42730, socket `/tmp/plugin-controllers.sock` (42700 tray, 42710 lights, 42720 ide-checkout are taken).
- Build gates before any commit: `cargo build -p plugin-controllers && cargo test -p plugin-controllers && cargo fmt --check -p plugin-controllers && cargo clippy -p plugin-controllers -- -D warnings`.

---

### Task 1: Scaffold crate from plugin-template

**Files:**
- Create: `plugins/plugin-controllers/` (Cargo.toml, plugin.toml, build.rs, Makefile, .gitignore, src/main.rs, src/lib.rs, src/platform/mod.rs)

**Interfaces:**
- Produces: crate `plugin-controllers` with `PLUGIN_ID` via `env!("QOL_PLUGIN_ID")`, `platform::open_settings()`.

- [ ] **Step 1: Copy template and prune**

```bash
cp -r plugins/plugin-template plugins/plugin-controllers
rm -rf plugins/plugin-controllers/.github plugins/plugin-controllers/LICENSE
```

- [ ] **Step 2: Rewrite Cargo.toml**

```toml
[package]
name = "plugin-controllers"
version = "0.1.0"
edition = "2021"

[dependencies]
anyhow = "1"
serde_json = "1"
qol-conventions.workspace = true
qol-plugin-daemon.workspace = true
qol-headless.workspace = true
open.workspace = true

[build-dependencies]
qol-conventions.workspace = true

[dev-dependencies]
qol-plugin-api.workspace = true
tempfile = "3"
```

(If `tempfile` is not already a workspace-visible dev dependency elsewhere, keep it crate-local as above; check `grep -rn 'tempfile' */*/Cargo.toml` and reuse the workspace form if one exists.)

- [ ] **Step 3: Rewrite plugin.toml**

Generate the uid: `uuidgen | tr A-F a-f`.

```toml
[plugin]
id = "plugin-controllers"
uid = "<uuidgen output>"
name = "Controllers"
description = "Detect game controllers with known driver defects and apply fixes"
version = "0.1.0"
author = "KMRH47"
platforms = ["linux"]

[runtime]
command = "plugin-controllers"

[action.apply_fixes]
label = "Apply Controller Fixes"
args = ["apply_fixes"]

[action.settings]
label = "Settings"
kind = "settings"
args = ["settings"]

[daemon]
enabled = true
command = "plugin-controllers"
socket = "/tmp/plugin-controllers.sock"
port = 42730

[menu]
label = "Controllers"
items = []

[[dependencies.binaries]]
name = "plugin-controllers"
repo = "qol-tools/plugin-controllers"
pattern = "plugin-controllers-{os}-{arch}"
```

- [ ] **Step 4: Update Makefile BINARY name to `plugin-controllers`; leave build.rs as-is**

- [ ] **Step 5: Minimal src/lib.rs + src/main.rs**

`src/lib.rs`:

```rust
pub mod platform;

pub const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
```

`src/main.rs`:

```rust
use std::process::ExitCode;

fn main() -> ExitCode {
    println!("plugin-controllers scaffold");
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::PluginManifest;

    #[test]
    fn validate_plugin_contract() {
        PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
    }
}
```

`src/platform/mod.rs`: keep template version (settings_url + open).

- [ ] **Step 6: Build gates, then commit**

Run: `cargo build -p plugin-controllers && cargo test -p plugin-controllers`
Expected: build OK, `validate_plugin_contract` PASS.

```bash
git add plugins/plugin-controllers
git commit -m "feat(plugin-controllers): scaffold plugin from template"
```

---

### Task 2: Fix database and matching (`src/fixes.rs`)

**Files:**
- Create: `plugins/plugin-controllers/src/fixes.rs`
- Modify: `src/lib.rs` (add `pub mod fixes;`)

**Interfaces:**
- Produces:
  - `pub struct Mac([u8; 6])` with `Mac::parse(&str) -> Option<Mac>` and lowercase-colon `Display`.
  - `pub struct FixEntry { pub id, pub summary, pub driver, pub bus, pub vendor, pub products, pub name, pub quirk_value }` (all `'static`; `bus: u16`, `vendor: u16`, `products: &'static [u16]`, `quirk_value: u16`).
  - `pub const FIXES: &[FixEntry]` containing `gulikit-xw-bt-rumble` (driver `hid_xpadneo`, bus `0x0005`, vendor `0x045e`, products `[0x02e0, 0x028e]`, name `GuliKit Controller XW`, quirk_value `263`).
  - `pub struct FixTarget { pub entry: &'static FixEntry, pub mac: Mac }` and `pub fn match_devices(devices: &[DetectedDevice]) -> Vec<FixTarget>` (deduped by entry id + mac).
- Consumes: `DetectedDevice` from Task 3 — to avoid a forward dependency, define `DetectedDevice` here in `fixes.rs`: `pub struct DetectedDevice { pub bus: u16, pub vendor: u16, pub product: u16, pub name: String, pub uniq: Option<String> }`.

- [ ] **Step 1: Write failing tests** (table-driven)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_parsing_accepts_only_six_hex_pairs() {
        let cases = [
            ("06:71:10:20:26:b4", Some("06:71:10:20:26:b4")),
            ("06:71:10:20:26:B4", Some("06:71:10:20:26:b4")),
            ("06:71:10:20:26", None),
            ("gg:71:10:20:26:b4", None),
            ("", None),
        ];
        for (input, expected) in cases {
            assert_eq!(
                Mac::parse(input).map(|m| m.to_string()),
                expected.map(str::to_string),
                "input: {input}"
            );
        }
    }

    fn device(bus: u16, vendor: u16, product: u16, name: &str, uniq: Option<&str>) -> DetectedDevice {
        DetectedDevice { bus, vendor, product, name: name.into(), uniq: uniq.map(str::to_string) }
    }

    #[test]
    fn matching_selects_known_pads_and_dedupes() {
        let gulikit = device(0x0005, 0x045e, 0x028e, "GuliKit Controller XW", Some("06:71:10:20:26:b4"));
        let gulikit_alt_pid = device(0x0005, 0x045e, 0x02e0, "GuliKit Controller XW", Some("06:71:10:20:26:b4"));
        let usb_clone = device(0x0003, 0x045e, 0x028e, "GuliKit Controller XW", Some("06:71:10:20:26:b4"));
        let no_mac = device(0x0005, 0x045e, 0x028e, "GuliKit Controller XW", None);
        let other = device(0x0005, 0x054c, 0x0ce6, "foo pad", Some("aa:bb:cc:dd:ee:ff"));

        let cases: [(&str, Vec<DetectedDevice>, usize); 4] = [
            ("single match", vec![gulikit.clone()], 1),
            ("same pad twice dedupes", vec![gulikit.clone(), gulikit_alt_pid], 1),
            ("usb transport and unknown pad ignored", vec![usb_clone, other], 0),
            ("missing mac ignored", vec![no_mac], 0),
        ];
        for (label, devices, expected) in cases {
            let targets = match_devices(&devices);
            assert_eq!(targets.len(), expected, "case: {label}");
        }
        let targets = match_devices(&[gulikit]);
        assert_eq!(targets[0].entry.id, "gulikit-xw-bt-rumble");
        assert_eq!(targets[0].mac.to_string(), "06:71:10:20:26:b4");
    }
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p plugin-controllers fixes` fails: module missing.

- [ ] **Step 3: Implement**

```rust
use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Mac([u8; 6]);

impl Mac {
    pub fn parse(input: &str) -> Option<Mac> {
        let mut bytes = [0u8; 6];
        let mut parts = input.split(':');
        for byte in &mut bytes {
            let part = parts.next()?;
            if part.len() != 2 {
                return None;
            }
            *byte = u8::from_str_radix(part, 16).ok()?;
        }
        match parts.next() {
            None => Some(Mac(bytes)),
            Some(_) => None,
        }
    }
}

impl fmt::Display for Mac {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [a, b, c, d, e, g] = self.0;
        write!(f, "{a:02x}:{b:02x}:{c:02x}:{d:02x}:{e:02x}:{g:02x}")
    }
}

pub struct DetectedDevice {
    pub bus: u16,
    pub vendor: u16,
    pub product: u16,
    pub name: String,
    pub uniq: Option<String>,
}

pub struct FixEntry {
    pub id: &'static str,
    pub summary: &'static str,
    pub driver: &'static str,
    pub bus: u16,
    pub vendor: u16,
    pub products: &'static [u16],
    pub name: &'static str,
    pub quirk_value: u16,
}

pub const FIXES: &[FixEntry] = &[FixEntry {
    id: "gulikit-xw-bt-rumble",
    summary: "GuliKit pad rumbles forever over Bluetooth without xpadneo quirk 263",
    driver: "hid_xpadneo",
    bus: 0x0005,
    vendor: 0x045e,
    products: &[0x02e0, 0x028e],
    name: "GuliKit Controller XW",
    quirk_value: 263,
}];

pub struct FixTarget {
    pub entry: &'static FixEntry,
    pub mac: Mac,
}

pub fn match_devices(devices: &[DetectedDevice]) -> Vec<FixTarget> {
    let mut targets: Vec<FixTarget> = Vec::new();
    for device in devices {
        for entry in FIXES {
            if device.bus != entry.bus
                || device.vendor != entry.vendor
                || !entry.products.contains(&device.product)
                || device.name != entry.name
            {
                continue;
            }
            let Some(mac) = device.uniq.as_deref().and_then(Mac::parse) else {
                continue;
            };
            let duplicate = targets
                .iter()
                .any(|t| t.entry.id == entry.id && t.mac == mac);
            if !duplicate {
                targets.push(FixTarget { entry, mac });
            }
        }
    }
    targets
}
```

Add `#[derive(Clone)]` on `DetectedDevice` (tests clone it).

- [ ] **Step 4: Run gates** — build, test, fmt, clippy all green.

- [ ] **Step 5: Commit** — `feat(plugin-controllers): fix database and controller matching`

---

### Task 3: /proc/bus/input/devices parser (`src/detect.rs`)

**Files:**
- Create: `plugins/plugin-controllers/src/detect.rs`
- Modify: `src/lib.rs` (add `pub mod detect;`)

**Interfaces:**
- Consumes: `fixes::DetectedDevice`.
- Produces: `pub fn parse_devices(text: &str) -> Vec<DetectedDevice>` and `pub fn read_devices() -> Vec<DetectedDevice>` (reads `/proc/bus/input/devices`, empty on any error).

- [ ] **Step 1: Write failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
I: Bus=0005 Vendor=045e Product=028e Version=1130
N: Name=\"GuliKit Controller XW\"
P: Phys=10:91:d1:28:d5:a6
U: Uniq=06:71:10:20:26:b4
H: Handlers=event27 js0

I: Bus=0003 Vendor=28de Product=11ff Version=0001
N: Name=\"Microsoft X-Box 360 pad 0\"
U: Uniq=
H: Handlers=event30 js1
";

    #[test]
    fn parser_extracts_bus_ids_name_and_uniq() {
        let devices = parse_devices(SAMPLE);
        assert_eq!(devices.len(), 2, "expected two device blocks");
        let first = &devices[0];
        assert_eq!(first.bus, 0x0005);
        assert_eq!(first.vendor, 0x045e);
        assert_eq!(first.product, 0x028e);
        assert_eq!(first.name, "GuliKit Controller XW");
        assert_eq!(first.uniq.as_deref(), Some("06:71:10:20:26:b4"));
        let second = &devices[1];
        assert_eq!(second.uniq, None, "empty Uniq= must map to None");
    }

    #[test]
    fn parser_skips_malformed_blocks() {
        let cases = [
            ("no I line", "N: Name=\"foo\"\n", 0),
            ("garbage ids", "I: Bus=zz Vendor=045e Product=028e Version=1130\nN: Name=\"foo\"\n", 0),
            ("missing name", "I: Bus=0005 Vendor=045e Product=028e Version=1130\n", 0),
        ];
        for (label, text, expected) in cases {
            assert_eq!(parse_devices(text).len(), expected, "case: {label}");
        }
    }
}
```

- [ ] **Step 2: Run to verify failure**

- [ ] **Step 3: Implement**

```rust
use crate::fixes::DetectedDevice;

pub fn read_devices() -> Vec<DetectedDevice> {
    std::fs::read_to_string("/proc/bus/input/devices")
        .map(|text| parse_devices(&text))
        .unwrap_or_default()
}

pub fn parse_devices(text: &str) -> Vec<DetectedDevice> {
    text.split("\n\n").filter_map(parse_block).collect()
}

fn parse_block(block: &str) -> Option<DetectedDevice> {
    let mut ids: Option<(u16, u16, u16)> = None;
    let mut name: Option<String> = None;
    let mut uniq: Option<String> = None;
    for line in block.lines() {
        if let Some(rest) = line.strip_prefix("I: ") {
            ids = parse_ids(rest);
        } else if let Some(rest) = line.strip_prefix("N: Name=") {
            name = Some(rest.trim_matches('"').to_string());
        } else if let Some(rest) = line.strip_prefix("U: Uniq=") {
            let value = rest.trim();
            uniq = (!value.is_empty()).then(|| value.to_string());
        }
    }
    let (bus, vendor, product) = ids?;
    Some(DetectedDevice { bus, vendor, product, name: name?, uniq })
}

fn parse_ids(rest: &str) -> Option<(u16, u16, u16)> {
    let mut bus = None;
    let mut vendor = None;
    let mut product = None;
    for field in rest.split_whitespace() {
        let (key, value) = field.split_once('=')?;
        let parsed = u16::from_str_radix(value, 16).ok();
        match key {
            "Bus" => bus = parsed,
            "Vendor" => vendor = parsed,
            "Product" => product = parsed,
            _ => {}
        }
    }
    Some((bus?, vendor?, product?))
}
```

Note: `parse_ids` returns `None` on unparseable hex via the `?` on `split_once` only; the `bus?`/`vendor?`/`product?` at the end covers garbage values because `parsed` stays `None`.

- [ ] **Step 4: Run gates** - all green.

- [ ] **Step 5: Commit** - `feat(plugin-controllers): parse input device table`

---

### Task 4: Fix-state computation (`src/state.rs`)

**Files:**
- Create: `plugins/plugin-controllers/src/state.rs`
- Modify: `src/lib.rs` (add `pub mod state;`)

**Interfaces:**
- Consumes: `fixes::{FixTarget, Mac}`.
- Produces:
  - `pub enum FixState { DriverMissing, Pending, LiveOnly, Applied }` (derive `Debug, Clone, Copy, PartialEq, Eq`).
  - `pub fn desired_quirk(target: &FixTarget) -> String` returning `"<mac>:<quirk_value>"`.
  - `pub struct SystemPaths { pub modprobe_dir: PathBuf, pub sys_module_dir: PathBuf }` with `SystemPaths::real()` returning `/etc/modprobe.d` and `/sys/module`.
  - `pub fn compute(paths: &SystemPaths, target: &FixTarget, driver_installed: bool) -> FixState`.
- Semantics: `Applied` = quirk string found on an `options` line for the driver in ANY `*.conf` under modprobe_dir (driver name matched with `-`/`_` normalized). `LiveOnly` = not persisted but present in `<sys_module_dir>/<driver>/parameters/quirks`. `DriverMissing` wins over everything when `driver_installed` is false.

- [ ] **Step 1: Write failing test** (uses `tempfile::TempDir` to build fake roots)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixes::{match_devices, DetectedDevice};
    use std::fs;

    fn target() -> crate::fixes::FixTarget {
        let device = DetectedDevice {
            bus: 0x0005,
            vendor: 0x045e,
            product: 0x028e,
            name: "GuliKit Controller XW".into(),
            uniq: Some("06:71:10:20:26:b4".into()),
        };
        match_devices(&[device]).remove(0)
    }

    #[test]
    fn desired_quirk_formats_mac_and_value() {
        assert_eq!(desired_quirk(&target()), "06:71:10:20:26:b4:263");
    }

    #[test]
    fn state_reflects_filesystem() {
        let cases = [
            ("nothing anywhere", None, None, true, FixState::Pending),
            ("driver missing wins", None, None, false, FixState::DriverMissing),
            (
                "persisted in any conf",
                Some("options hid_xpadneo quirks=06:71:10:20:26:b4:263"),
                None,
                true,
                FixState::Applied,
            ),
            (
                "persisted with dash driver name",
                Some("options hid-xpadneo quirks=06:71:10:20:26:b4:263"),
                None,
                true,
                FixState::Applied,
            ),
            (
                "live only",
                None,
                Some("06:71:10:20:26:b4:263"),
                true,
                FixState::LiveOnly,
            ),
            (
                "other mac does not count",
                Some("options hid_xpadneo quirks=aa:bb:cc:dd:ee:ff:263"),
                None,
                true,
                FixState::Pending,
            ),
        ];
        for (label, conf_line, sysfs_value, driver_installed, expected) in cases {
            let root = tempfile::tempdir().expect("tempdir");
            let modprobe_dir = root.path().join("modprobe.d");
            let sys_module_dir = root.path().join("module");
            fs::create_dir_all(&modprobe_dir).expect("mkdir modprobe");
            if let Some(line) = conf_line {
                fs::write(modprobe_dir.join("a.conf"), format!("{line}\n")).expect("conf");
            }
            if let Some(value) = sysfs_value {
                let params = sys_module_dir.join("hid_xpadneo/parameters");
                fs::create_dir_all(&params).expect("mkdir params");
                fs::write(params.join("quirks"), value).expect("quirks");
            }
            let paths = SystemPaths { modprobe_dir, sys_module_dir };
            assert_eq!(compute(&paths, &target(), driver_installed), expected, "case: {label}");
        }
    }
}
```

- [ ] **Step 2: Run to verify failure**

- [ ] **Step 3: Implement**

```rust
use std::path::PathBuf;

use crate::fixes::FixTarget;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixState {
    DriverMissing,
    Pending,
    LiveOnly,
    Applied,
}

pub struct SystemPaths {
    pub modprobe_dir: PathBuf,
    pub sys_module_dir: PathBuf,
}

impl SystemPaths {
    pub fn real() -> SystemPaths {
        SystemPaths {
            modprobe_dir: PathBuf::from("/etc/modprobe.d"),
            sys_module_dir: PathBuf::from("/sys/module"),
        }
    }
}

pub fn desired_quirk(target: &FixTarget) -> String {
    format!("{}:{}", target.mac, target.entry.quirk_value)
}

pub fn compute(paths: &SystemPaths, target: &FixTarget, driver_installed: bool) -> FixState {
    if !driver_installed {
        return FixState::DriverMissing;
    }
    let quirk = desired_quirk(target);
    if persisted(paths, target.entry.driver, &quirk) {
        return FixState::Applied;
    }
    if live(paths, target.entry.driver, &quirk) {
        return FixState::LiveOnly;
    }
    FixState::Pending
}

fn persisted(paths: &SystemPaths, driver: &str, quirk: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(&paths.modprobe_dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("conf") {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        if contents.lines().any(|line| options_line_has_quirk(line, driver, quirk)) {
            return true;
        }
    }
    false
}

fn options_line_has_quirk(line: &str, driver: &str, quirk: &str) -> bool {
    let normalized = line.trim().replace('-', "_");
    normalized.starts_with("options")
        && normalized.contains(&driver.replace('-', "_"))
        && normalized.contains(quirk)
}

fn live(paths: &SystemPaths, driver: &str, quirk: &str) -> bool {
    let param = paths
        .sys_module_dir
        .join(driver)
        .join("parameters/quirks");
    std::fs::read_to_string(param)
        .map(|value| value.contains(quirk))
        .unwrap_or(false)
}
```

- [ ] **Step 4: Run gates** - all green.

- [ ] **Step 5: Commit** - `feat(plugin-controllers): compute per-pad fix state`

---

### Task 5: Apply action (`src/apply.rs`)

**Files:**
- Create: `plugins/plugin-controllers/src/apply.rs`
- Modify: `src/lib.rs` (add `pub mod apply;`)

**Interfaces:**
- Consumes: `fixes::FixTarget`, `state::desired_quirk`.
- Produces:
  - `pub fn conf_contents(targets: &[FixTarget]) -> String` - regenerated full contents of `/etc/modprobe.d/qol-controllers.conf`: one `options <driver> quirks=<q1>,<q2>` line per driver, drivers and quirks sorted for determinism.
  - `pub fn sysfs_writes(targets: &[FixTarget]) -> Vec<(String, String)>` - per driver: (`/sys/module/<driver>/parameters/quirks`, comma-joined quirks).
  - `pub fn apply(targets: &[FixTarget]) -> anyhow::Result<()>` - runs pkexec once with a fixed `sh -c` script taking the conf contents and sysfs pairs as positional arguments (no string interpolation of device data into the script).
- Safety: quirk strings are derived from `Mac` (parsed) and a `u16`, so they cannot contain shell metacharacters; they are still passed as argv, never spliced into the script.

- [ ] **Step 1: Write failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixes::{match_devices, DetectedDevice};

    fn targets() -> Vec<crate::fixes::FixTarget> {
        let device = DetectedDevice {
            bus: 0x0005,
            vendor: 0x045e,
            product: 0x028e,
            name: "GuliKit Controller XW".into(),
            uniq: Some("06:71:10:20:26:b4".into()),
        };
        match_devices(&[device])
    }

    #[test]
    fn conf_contents_regenerates_whole_file() {
        let expected = "options hid_xpadneo quirks=06:71:10:20:26:b4:263\n";
        assert_eq!(conf_contents(&targets()), expected);
    }

    #[test]
    fn sysfs_writes_target_driver_param() {
        let writes = sysfs_writes(&targets());
        assert_eq!(
            writes,
            vec![(
                "/sys/module/hid_xpadneo/parameters/quirks".to_string(),
                "06:71:10:20:26:b4:263".to_string()
            )]
        );
    }
}
```

- [ ] **Step 2: Run to verify failure**

- [ ] **Step 3: Implement**

```rust
use std::collections::BTreeMap;
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::fixes::FixTarget;
use crate::state::desired_quirk;

const CONF_PATH: &str = "/etc/modprobe.d/qol-controllers.conf";

fn quirks_by_driver(targets: &[FixTarget]) -> BTreeMap<&'static str, Vec<String>> {
    let mut map: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    for target in targets {
        let quirks = map.entry(target.entry.driver).or_default();
        let quirk = desired_quirk(target);
        if !quirks.contains(&quirk) {
            quirks.push(quirk);
        }
    }
    for quirks in map.values_mut() {
        quirks.sort();
    }
    map
}

pub fn conf_contents(targets: &[FixTarget]) -> String {
    quirks_by_driver(targets)
        .iter()
        .map(|(driver, quirks)| format!("options {driver} quirks={}\n", quirks.join(",")))
        .collect()
}

pub fn sysfs_writes(targets: &[FixTarget]) -> Vec<(String, String)> {
    quirks_by_driver(targets)
        .iter()
        .map(|(driver, quirks)| {
            (
                format!("/sys/module/{driver}/parameters/quirks"),
                quirks.join(","),
            )
        })
        .collect()
}

pub fn apply(targets: &[FixTarget]) -> Result<()> {
    if targets.is_empty() {
        bail!("no known controllers connected");
    }
    let conf = conf_contents(targets);
    let script = r#"set -e
printf '%s' "$1" > /etc/modprobe.d/qol-controllers.conf
shift
while [ "$#" -ge 2 ]; do
  if [ -e "$1" ]; then printf '%s' "$2" > "$1"; fi
  shift 2
done"#;
    let mut command = Command::new("pkexec");
    command.args(["sh", "-c", script, "qol-controllers", &conf]);
    for (path, value) in sysfs_writes(targets) {
        command.arg(path).arg(value);
    }
    let status = command.status().context("failed to launch pkexec")?;
    if !status.success() {
        bail!("pkexec exited with {status}");
    }
    Ok(())
}
```

(`CONF_PATH` is referenced only in the script string; if clippy flags it unused, inline-drop the const.)

- [ ] **Step 4: Run gates** - all green.

- [ ] **Step 5: Commit** - `feat(plugin-controllers): compose and apply privileged fixes`

---

### Task 6: Daemon (`src/daemon.rs` + `src/platform` driver check)

**Files:**
- Create: `plugins/plugin-controllers/src/daemon.rs`
- Modify: `src/lib.rs` (add `pub mod daemon;`), `src/platform/mod.rs` (add `driver_installed`)

**Interfaces:**
- Consumes: `detect::read_devices`, `fixes::match_devices`, `state::{compute, FixState, SystemPaths}`, `apply::apply`, `qol_plugin_daemon::{daemon, notification}`, `platform::{open_settings, driver_installed}`.
- Produces: `pub fn run_from_env() -> Result<()>`, `pub fn execute_action_once(action: &str) -> Result<()>`, `pub fn snapshot() -> Vec<TargetStatus>` where `pub struct TargetStatus { pub fix_id: &'static str, pub mac: String, pub summary: &'static str, pub state: FixState }`.

- [ ] **Step 1: Add `driver_installed` to platform/mod.rs**

```rust
pub fn driver_installed(driver: &str) -> bool {
    let module_name = driver.replace('_', "-");
    if std::path::Path::new("/sys/module").join(driver).exists() {
        return true;
    }
    std::process::Command::new("modinfo")
        .args(["-n", &module_name])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
```

- [ ] **Step 2: Write dispatch test** (state machine only, no socket)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_dispatch_recognizes_known_actions() {
        let cases = [
            ("apply_fixes", true),
            ("settings", true),
            ("status", true),
            ("bogus", false),
        ];
        for (action, known) in cases {
            assert_eq!(is_supported_action(action), known, "action: {action}");
        }
    }
}
```

- [ ] **Step 3: Implement daemon**

```rust
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_plugin_daemon::notification::send_notification;

use crate::fixes::match_devices;
use crate::state::{compute, FixState, SystemPaths};
use crate::{apply, detect, platform};

const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

const POLL_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Clone)]
pub struct TargetStatus {
    pub fix_id: &'static str,
    pub mac: String,
    pub summary: &'static str,
    pub state: FixState,
}

pub fn snapshot() -> Vec<TargetStatus> {
    let paths = SystemPaths::real();
    let devices = detect::read_devices();
    match_devices(&devices)
        .iter()
        .map(|target| TargetStatus {
            fix_id: target.entry.id,
            mac: target.mac.to_string(),
            summary: target.entry.summary,
            state: compute(&paths, target, platform::driver_installed(target.entry.driver)),
        })
        .collect()
}

pub fn run_from_env() -> Result<()> {
    let notified: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let poll_notified = Arc::clone(&notified);
    thread::spawn(move || poll_loop(poll_notified));
    core_daemon::run_stateful_listener(&DAEMON_CONFIG, (), |_, action| handle_action(action))
        .context("plugin-controllers daemon listener failed")
}

fn poll_loop(notified: Arc<Mutex<Vec<String>>>) {
    loop {
        for status in snapshot() {
            let key = format!("{}/{}", status.fix_id, status.mac);
            let mut seen = notified.lock().expect("notified lock");
            if seen.contains(&key) {
                continue;
            }
            match status.state {
                FixState::Pending | FixState::LiveOnly => {
                    seen.push(key);
                    send_notification(
                        "Controller fix available",
                        &format!("{} - run Apply Controller Fixes", status.summary),
                    );
                }
                FixState::DriverMissing => {
                    seen.push(key);
                    send_notification(
                        "Controller driver missing",
                        &format!("{} - install xpadneo first", status.summary),
                    );
                }
                FixState::Applied => {}
            }
        }
        thread::sleep(POLL_INTERVAL);
    }
}

fn is_supported_action(action: &str) -> bool {
    matches!(action, "apply_fixes" | "settings" | "status")
}

fn handle_action(action: &str) -> ReadResult<()> {
    match action {
        "apply_fixes" => match apply_pending() {
            Ok(message) => {
                send_notification("Controller fixes", &message);
                ReadResult::Handled
            }
            Err(error) => ReadResult::Error(format!("{error:#}")),
        },
        "settings" => match platform::open_settings() {
            Ok(()) => ReadResult::Handled,
            Err(error) => ReadResult::Error(format!("{error:#}")),
        },
        "status" => ReadResult::HandledWithData(status_json()),
        _ => ReadResult::Error(format!("unknown action: {action}")),
    }
}

fn status_json() -> serde_json::Value {
    let statuses: Vec<serde_json::Value> = snapshot()
        .iter()
        .map(|status| {
            serde_json::json!({
                "fix": status.fix_id,
                "mac": status.mac,
                "state": format!("{:?}", status.state),
            })
        })
        .collect();
    serde_json::json!({ "targets": statuses })
}

fn apply_pending() -> Result<String> {
    let paths = SystemPaths::real();
    let devices = detect::read_devices();
    let targets = match_devices(&devices);
    if targets.is_empty() {
        bail!("no known controllers connected");
    }
    let actionable: Vec<_> = targets
        .iter()
        .filter(|target| {
            let installed = platform::driver_installed(target.entry.driver);
            compute(&paths, target, installed) != FixState::DriverMissing && installed
        })
        .cloned()
        .collect();
    if actionable.is_empty() {
        bail!("driver missing; install xpadneo: git clone https://github.com/atar-axis/xpadneo && cd xpadneo && sudo ./install.sh");
    }
    apply::apply(&actionable)?;
    Ok(format!("applied {} fix(es)", actionable.len()))
}

pub fn execute_action_once(action: &str) -> Result<()> {
    if !is_supported_action(action) {
        bail!("unknown action: {action}");
    }
    match handle_action(action) {
        ReadResult::Handled => Ok(()),
        ReadResult::HandledWithData(data) => {
            println!("{data}");
            Ok(())
        }
        ReadResult::Error(message) => bail!(message),
        _ => Ok(()),
    }
}
```

`FixTarget` needs `#[derive(Clone)]` (and `FixEntry` references are `&'static`, so a plain derive works); add it in fixes.rs.

Check `run_stateful_listener`'s exact handler signature (`FnMut(&mut S, &str) -> ReadResult<()>` with `S = ()`); adjust the closure if the real signature differs.

- [ ] **Step 4: Run gates** - all green.

- [ ] **Step 5: Commit** - `feat(plugin-controllers): daemon with poll loop, notifications, and actions`

---

### Task 7: CLI, doctor, main dispatch, final verification

**Files:**
- Create: `plugins/plugin-controllers/src/cli.rs`
- Modify: `src/lib.rs` (add `pub mod cli;`), `src/main.rs` (real dispatch)

**Interfaces:**
- Consumes: everything above plus `qol_headless::{Command, DoctorCheck, DoctorCheckResult, HeadlessApp, PlainTextOutput}` (mirror qol-shot's `cli.rs` usage for constructor details).
- Produces: `pub fn exit_code(args) -> ExitCode`.

- [ ] **Step 1: Implement cli.rs**

```rust
use std::process::ExitCode;

use anyhow::Result;
use qol_headless::{Command, DoctorCheck, DoctorCheckResult, HeadlessApp, PlainTextOutput};

use crate::state::FixState;
use crate::{daemon, PLUGIN_ID};

const BINARY_NAME: &str = "plugin-controllers";

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
        .about("Detect game controllers with known driver defects and apply fixes.")
        .command(apply_command())
        .command(status_command())
        .command(settings_command())
        .doctor_checks(doctor_checks())
}

fn apply_command() -> Command {
    Command::new("apply_fixes")
        .about("Apply fixes for connected known-broken controllers (one pkexec prompt).")
        .run_plain_text(|_| {
            daemon::execute_action_once("apply_fixes")?;
            Ok(PlainTextOutput::default())
        })
}

fn status_command() -> Command {
    Command::new("status")
        .about("Print detected known controllers and their fix state.")
        .run_plain_text(|_| {
            daemon::execute_action_once("status")?;
            Ok(PlainTextOutput::default())
        })
}

fn settings_command() -> Command {
    Command::new("settings")
        .about("Open the plugin settings page.")
        .run_plain_text(|_| {
            crate::platform::open_settings()?;
            Ok(PlainTextOutput::default())
        })
}

fn doctor_checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            "pkexec_available",
            "Verify pkexec exists for the privileged apply step.",
            pkexec_check,
        ),
        DoctorCheck::new(
            "controller_fixes",
            "Verify connected known controllers have their fixes applied.",
            fixes_check,
        ),
    ]
}

fn pkexec_check() -> Result<DoctorCheckResult> {
    let found = std::process::Command::new("pkexec")
        .arg("--version")
        .output()
        .is_ok();
    Ok(if found {
        DoctorCheckResult::ok("pkexec_available", "pkexec found")
    } else {
        DoctorCheckResult::warn("pkexec_available", "pkexec not found")
            .with_fix("install polkit (provides pkexec)")
    })
}

fn fixes_check() -> Result<DoctorCheckResult> {
    let statuses = daemon::snapshot();
    if statuses.is_empty() {
        return Ok(DoctorCheckResult::ok(
            "controller_fixes",
            format!("no known controllers connected ({} fixes in database)", crate::fixes::FIXES.len()),
        ));
    }
    let mut worst = DoctorCheckResult::ok("controller_fixes", summary_line(&statuses));
    for status in &statuses {
        match status.state {
            FixState::DriverMissing => {
                return Ok(DoctorCheckResult::fail("controller_fixes", summary_line(&statuses))
                    .with_fix("install xpadneo: https://github.com/atar-axis/xpadneo"));
            }
            FixState::Pending | FixState::LiveOnly => {
                worst = DoctorCheckResult::warn("controller_fixes", summary_line(&statuses))
                    .with_fix(format!("run: {BINARY_NAME} apply_fixes"));
            }
            FixState::Applied => {}
        }
    }
    Ok(worst)
}

fn summary_line(statuses: &[daemon::TargetStatus]) -> String {
    statuses
        .iter()
        .map(|s| format!("{} [{}]: {:?}", s.fix_id, s.mac, s.state))
        .collect::<Vec<_>>()
        .join("; ")
}
```

Adjust constructor/method names to the real qol-headless API while implementing (`.usage()`, `.output()` etc. optional; qol-shot `cli.rs` is the reference).

- [ ] **Step 2: Rewrite main.rs dispatch** (qol-shot pattern)

```rust
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() && std::env::var_os(qol_conventions::ENV_DAEMON_SOCKET).is_some() {
        return match plugin_controllers::daemon::run_from_env() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{error:#}");
                ExitCode::from(1)
            }
        };
    }
    plugin_controllers::cli::exit_code(args)
}

#[cfg(test)]
mod tests {
    use qol_plugin_api::manifest::PluginManifest;

    #[test]
    fn validate_plugin_contract() {
        PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
    }
}
```

- [ ] **Step 3: Full gates** - `cargo build -p plugin-controllers && cargo test -p plugin-controllers && cargo fmt --check && cargo clippy -p plugin-controllers -- -D warnings`

- [ ] **Step 4: Live verification on this machine (GuliKit pad connected)**

Run: `./target/debug/plugin-controllers status`
Expected: one target, `gulikit-xw-bt-rumble`, state `Applied` (this PC already has the quirk persisted in `99-xpadneo-joystick.conf`).

Run: `./target/debug/plugin-controllers doctor`
Expected: `pkexec_available` ok, `controller_fixes` ok.

- [ ] **Step 5: Commit** - `feat(plugin-controllers): cli, doctor checks, and daemon dispatch`

---

## Self-Review Notes

- Spec coverage: fix database (Task 2), detection (Task 3), read-only state (Task 4), pkexec apply (Task 5), daemon + notifications + never-escalate (Task 6), doctor + driver-missing guidance (Task 7). Platform section satisfied by std-only code that degrades to "no devices" off-Linux.
- The spec's `src/fixes/` (dir) layout is flattened to single files (`fixes.rs` etc.) since each module is small; same boundaries, fewer files.
- Type consistency: `FixState` variants used identically in Tasks 4, 6, 7; `desired_quirk` defined Task 4, consumed Task 5; `TargetStatus` defined Task 6, consumed Task 7.
- Real-device fixture text in Task 3 tests is intentional domain data (the GuliKit block is the bug being encoded); other tests use generic names.
