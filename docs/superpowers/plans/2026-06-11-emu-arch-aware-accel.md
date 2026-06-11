# Emu Arch-Aware Binary And Acceleration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Pick the QEMU binary, accelerator, machine type, and firmware from the guest architecture so hvf/kvm/whpx engage when host arch matches guest arch, completing M2 of `docs/superpowers/specs/2026-06-10-emu-test-harness-design.md`.

**Architecture:** A `GuestArch` enum is parsed at the discovery boundary (config may declare `arch` per image; libvirt/filesystem default to x86_64) and flows through `Environment` into resolution (`qemu-system-<arch>`, per-arch accel) and launch args (machine `q35` vs `virt`, `-cpu`, edk2 pflash for aarch64). Acceleration becomes a shared pure function `select(hypervisor, available, host_arch, guest)` with per-OS hypervisor facts.

**Tech Stack:** Rust, existing `qol emu` module (`tools/qol-cli/src/commands/emu/`), QEMU 11, toml crate.

**Out of scope:** whpx availability probing on Windows (assumed present like today), arches beyond x86_64/aarch64, guest arch auto-detection from image contents.

---

### Task 1: GuestArch enum threaded through Environment and resolution

**Files:**
- Create: `tools/qol-cli/src/commands/emu/arch.rs`
- Modify: `tools/qol-cli/src/commands/emu.rs` (Environment, EnvironmentStatus, resolve_environment, report json, qemu_args test fixture, `mod arch;`)
- Modify: `tools/qol-cli/src/commands/emu/discovery/config.rs:16`, `discovery/libvirt.rs:23`, `discovery/filesystem.rs:48`
- Modify: `tools/qol-cli/src/dev_console.rs:1141`

- [x] **Step 1: Write arch.rs with failing-to-compile consumers in mind**

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GuestArch {
    X86_64,
    Aarch64,
}

impl GuestArch {
    pub(crate) const ALL: [GuestArch; 2] = [GuestArch::X86_64, GuestArch::Aarch64];

    pub(crate) fn parse(value: &str) -> Option<GuestArch> {
        match value {
            "x86_64" => Some(GuestArch::X86_64),
            "aarch64" => Some(GuestArch::Aarch64),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            GuestArch::X86_64 => "x86_64",
            GuestArch::Aarch64 => "aarch64",
        }
    }

    pub(crate) fn qemu_system_binary(self) -> &'static str {
        match self {
            GuestArch::X86_64 => "qemu-system-x86_64",
            GuestArch::Aarch64 => "qemu-system-aarch64",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trips_supported_arches() {
        let cases = [
            ("x86_64", Some(GuestArch::X86_64)),
            ("aarch64", Some(GuestArch::Aarch64)),
            ("arm64", None),
            ("", None),
        ];
        for (input, expected) in cases {
            assert_eq!(GuestArch::parse(input), expected, "input: {input}");
        }
        for arch in GuestArch::ALL {
            assert_eq!(GuestArch::parse(arch.as_str()), Some(arch), "arch: {arch:?}");
        }
    }

    #[test]
    fn qemu_system_binary_is_arch_suffixed() {
        let cases = [
            (GuestArch::X86_64, "qemu-system-x86_64"),
            (GuestArch::Aarch64, "qemu-system-aarch64"),
        ];
        for (arch, expected) in cases {
            assert_eq!(arch.qemu_system_binary(), expected, "arch: {arch:?}");
        }
    }
}
```

- [x] **Step 2: Thread the enum through**

In `emu.rs`: add `mod arch;` and `use arch::GuestArch;`. Change `Environment.arch: GuestArch` and `EnvironmentStatus.arch: GuestArch`. In `resolve_environment` replace the hardcoded lookup:

```rust
let qemu_system = find_on_path(environment.arch.qemu_system_binary());
```

and the unsupported reason:

```rust
reason: format!("missing {}", environment.arch.qemu_system_binary()),
```

Report json: `"arch": input.environment.arch.as_str()`. Test fixture: `arch: GuestArch::X86_64`. Discovery sources: `arch: GuestArch::X86_64` (import via `use super::super::{arch::GuestArch, ...}` or matching path). Dev console: `status.arch.as_str()`.

- [x] **Step 3: Gate**

Run: `cargo fmt -p qol && cargo clippy -p qol --all-targets --all-features -- -D warnings && cargo test -p qol --all-features`
Expected: PASS

- [x] **Step 4: Commit**

```bash
git commit -m "feat(emu): resolve qemu binary from guest arch enum"
```

### Task 2: Config images accept table form with arch

**Files:**
- Modify: `tools/qol-cli/src/commands/emu/discovery/config.rs`

- [x] **Step 1: Write failing tests**

```rust
#[test]
fn parses_table_form_with_arch() {
    let overrides = parse_image_overrides(
        r#"
[images.foo]
path = "/a/b/foo.qcow2"
arch = "aarch64"
"#,
        None,
    )
    .unwrap();
    let (path, arch) = overrides.get("foo").unwrap();
    assert_eq!(path, &PathBuf::from("/a/b/foo.qcow2"));
    assert_eq!(*arch, GuestArch::Aarch64);
}

#[test]
fn string_form_defaults_to_x86_64() {
    let overrides = parse_image_overrides(r#"[images]
foo = "/a/b/foo.qcow2""#, None).unwrap();
    assert_eq!(overrides.get("foo").unwrap().1, GuestArch::X86_64);
}

#[test]
fn rejects_unknown_arch() {
    let error = parse_image_overrides(
        r#"
[images.foo]
path = "/a/b/foo.qcow2"
arch = "sparc"
"#,
        None,
    )
    .unwrap_err();
    assert!(error.to_string().contains("images.foo.arch"), "error: {error}");
}
```

- [x] **Step 2: Run tests, verify the new ones fail to compile/fail**

Run: `cargo test -p qol --all-features config`

- [x] **Step 3: Implement**

`load_image_overrides`/`parse_image_overrides` return `HashMap<String, (PathBuf, GuestArch)>`; per entry:

```rust
let (path, arch) = match value {
    TomlValue::String(path) => (path.as_str(), GuestArch::X86_64),
    TomlValue::Table(table) => {
        let path = table
            .get("path")
            .and_then(TomlValue::as_str)
            .ok_or_else(|| anyhow!("images.{id}.path must be a string path"))?;
        let arch = match table.get("arch") {
            None => GuestArch::X86_64,
            Some(value) => value
                .as_str()
                .and_then(GuestArch::parse)
                .ok_or_else(|| anyhow!("images.{id}.arch must be one of: x86_64, aarch64"))?,
        };
        (path, arch)
    }
    _ => bail!("images.{id} must be a string path or a table with path/arch"),
};
overrides.insert(sanitize_id(id), (expand_home(path, home), arch));
```

`discover` maps `(id, (image_path, arch))` into `Environment { arch, ... }`.

- [x] **Step 4: Gate (full command from Task 1 Step 3)**

- [x] **Step 5: Commit**

```bash
git commit -m "feat(emu): per-image guest arch in emu.toml"
```

### Task 3: Acceleration selected per guest arch, doctor shows per-arch rows

**Files:**
- Modify: `tools/qol-cli/src/commands/emu/platform/mod.rs`, `platform/macos.rs`, `platform/linux.rs`, `platform/windows.rs`
- Modify: `tools/qol-cli/src/commands/emu.rs` (resolve_environment, cmd_doctor)

- [x] **Step 1: Write failing select test in platform/mod.rs**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::emu::arch::GuestArch;

    #[test]
    fn select_accelerates_only_matching_available_arch() {
        let cases = [
            ("hvf", true, "aarch64", GuestArch::Aarch64, "hvf"),
            ("hvf", true, "aarch64", GuestArch::X86_64, "tcg"),
            ("kvm", false, "x86_64", GuestArch::X86_64, "tcg"),
            ("whpx", true, "x86_64", GuestArch::X86_64, "whpx"),
        ];
        for (hypervisor, available, host, guest, expected) in cases {
            assert_eq!(
                select(hypervisor, available, host, guest),
                expected,
                "hypervisor: {hypervisor}, host: {host}, guest: {guest:?}"
            );
        }
    }
}
```

- [x] **Step 2: Implement**

`platform/mod.rs`:

```rust
use super::arch::GuestArch;

pub(crate) fn acceleration(guest: GuestArch) -> &'static str {
    select(hypervisor(), hypervisor_available(), std::env::consts::ARCH, guest)
}

fn select(hypervisor: &'static str, available: bool, host_arch: &str, guest: GuestArch) -> &'static str {
    if available && host_arch == guest.as_str() {
        hypervisor
    } else {
        "tcg"
    }
}
```

Each OS module drops `acceleration()` and gains:

```rust
pub(crate) fn hypervisor() -> &'static str { "hvf" }
pub(crate) fn hypervisor_available() -> bool { true }
```

(linux: `"kvm"` / `Path::new("/dev/kvm").exists()`; windows: `"whpx"` / `true`.)

Callers: `resolve_environment` uses `platform::acceleration(environment.arch)`; `cmd_doctor` replaces the single `qemu`/`accel` rows with one row per `GuestArch::ALL`:

```rust
for arch in GuestArch::ALL {
    match find_on_path(arch.qemu_system_binary()) {
        Some(path) => step_label(
            arch.as_str(),
            StepKind::Found,
            &format!("{} · {}", path.display(), platform::acceleration(arch)),
        ),
        None => step_label(
            arch.as_str(),
            StepKind::Info,
            &format!("missing {}", arch.qemu_system_binary()),
        ),
    }
}
```

- [x] **Step 3: Gate**

- [x] **Step 4: Commit**

```bash
git commit -m "feat(emu): pick accelerator per guest arch"
```

### Task 4: aarch64 machine type, cpu, and edk2 firmware wiring

**Files:**
- Modify: `tools/qol-cli/src/commands/emu/arch.rs` (machine_type, firmware_file)
- Modify: `tools/qol-cli/src/commands/emu.rs` (Resolution.firmware, locate_firmware, resolve_environment, qemu_args + call site + tests)

- [x] **Step 1: Extend arch.rs**

```rust
pub(crate) fn machine_type(self) -> &'static str {
    match self {
        GuestArch::X86_64 => "q35",
        GuestArch::Aarch64 => "virt",
    }
}

pub(crate) fn firmware_file(self) -> Option<&'static str> {
    match self {
        GuestArch::X86_64 => None,
        GuestArch::Aarch64 => Some("edk2-aarch64-code.fd"),
    }
}
```

- [x] **Step 2: Write failing qemu_args tests**

aarch64 + hvf + firmware path: expect `-machine virt`, `-cpu host`, `-drive if=pflash,format=raw,readonly=on,file=/fw/edk2-aarch64-code.fd` before the disk drive. aarch64 + tcg: expect `-cpu max`. x86_64 case: unchanged args, no `-cpu`, no pflash.

- [x] **Step 3: Implement**

`locate_firmware`:

```rust
fn locate_firmware(qemu_system: &Path, arch: GuestArch) -> std::result::Result<Option<PathBuf>, String> {
    let Some(file) = arch.firmware_file() else {
        return Ok(None);
    };
    let Some(bin_dir) = qemu_system.parent() else {
        return Err(format!("{} has no parent directory", qemu_system.display()));
    };
    let candidate = bin_dir.join("../share/qemu").join(file);
    match candidate.canonicalize() {
        Ok(path) if path.is_file() => Ok(Some(path)),
        _ => Err(format!("missing {file} under {}", bin_dir.join("../share/qemu").display())),
    }
}
```

`Resolution` gains `firmware: Option<PathBuf>` (None in the two early-unsupported returns); after the qemu-img check, a firmware failure becomes `ResolveState::Unsupported` with the error string as reason. `qemu_args` gains `firmware: Option<&Path>`, uses `environment.arch.machine_type()`, and for `GuestArch::Aarch64` pushes `-cpu host|max` (max iff acceleration == "tcg") and the pflash drive. Call site passes `resolution.firmware.as_deref()`.

- [x] **Step 4: Gate**

- [x] **Step 5: Commit**

```bash
git commit -m "feat(emu): aarch64 virt machine with edk2 firmware"
```

### Task 5: Runtime verification on the arm64 host

- [x] **Step 1: Scratch aarch64 image + config**

```bash
qemu-img create -f qcow2 ~/VMs/scratch-arm.qcow2 1G
cat > "$(qol emu doctor | grep config | awk '{print $2}')" <<'EOF'
[images.scratch-arm]
path = "~/VMs/scratch-arm.qcow2"
arch = "aarch64"
EOF
qol emu doctor
```

Expected doctor: `x86_64 ... tcg`, `aarch64 ... hvf`. Note: the config path is
the platform config dir (macOS: `~/Library/Application Support/qol-tray/emu.toml`),
not `~/.config`; `qol emu doctor` prints it.

- [x] **Step 2: Full cycle**

`qol emu up scratch-arm` (background), then `shot`, `down`. Expect report `acceleration: "hvf"`, EDK2 boot screen in the screenshot, status `pass`, teardown removes images.

- [x] **Step 3: Clean up**

Delete `~/VMs/scratch-arm.qcow2`, restore/remove `emu.toml`, confirm `qol emu list` is back to prior state. (Verified config-driven discovery, resolution `ready`, and report fields before cleanup.)

### Task 6: Docs

**Files:**
- Modify: `apps/qol-tray/skills/qol-cli-commands/SKILL.md` (Emu section: arch-aware selection done, table config form, doctor rows)
- Modify: `docs/superpowers/specs/2026-06-10-emu-test-harness-design.md` (Status: M2 complete)

- [x] **Step 1: Update both docs, mark M2 complete, commit**

```bash
git commit -m "docs(emu): arch-aware accel completes m2"
```
