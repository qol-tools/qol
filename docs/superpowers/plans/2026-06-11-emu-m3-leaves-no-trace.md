# Emu M3 Leaves-No-Trace Workflow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** M3 of `docs/superpowers/specs/2026-06-10-emu-test-harness-design.md`: one GuestOs adapter (Debian nocloud aarch64), the Run verb facade, and a `qol emu check <id>` command that runs the `leaves-no-trace` workflow end-to-end and writes a verdict into report.json.

**Architecture:** The VM gains a second loopback socket (`-serial tcp`) carrying the guest console. A `SerialClient` drives it expect-style (wait for marker, send line, run command with an un-echoable rc marker). A `GuestOs` trait under `emu/guest/` seals Debian knowledge (root login, stick format/mount, stub provisioning, reboot, trace listing). `Run` exposes only verbs (insert/launch_qol/pull/reboot/list_traces) backed by QMP + serial; `leaves_no_trace` composes them into a Verdict. `cmd_check` reuses the boot pipeline extracted from `cmd_up`.

**Tech Stack:** Rust, QEMU 11 (hvf aarch64), Debian 13 nocloud arm64 qcow2, existing qmp.rs patterns.

**MVP deviations (documented in spec Status):** the medium stick is provisioned in-guest (mkfs + stub written over serial) instead of pre-loaded on the host; `list_qol_traces` runs `find` in-guest after reboot instead of host-reading a DiskSnapshot; the qol artifact is a stub script; the GuestOs impl is hardcoded to Debian (adapter registry is M4).

---

### Task 1: Serial console socket on every VM

**Files:**
- Modify: `tools/qol-cli/src/commands/emu.rs` (qemu_args + tests, cmd_up port probe, ReportInput/report_json serial field)

- [x] **Step 1: Extend qemu_args test**

In `qemu_args_wire_accel_display_and_qmp`, pass a new `serial_port` argument (5555) and assert the fragment `-serial tcp:127.0.0.1:5555,server,nowait`.

- [x] **Step 2: Implement**

`qemu_args(..., qmp_port: u16, serial_port: u16, firmware: Option<&Path>)` appends after the `-qmp` pair:

```rust
        "-serial".to_string(),
        format!("tcp:127.0.0.1:{serial_port},server,nowait"),
```

`cmd_up` probes `let serial_port = machine::free_qmp_port()?;` and every `qmp: Some(json!(...))` report gains a sibling field by extending `ReportInput` with `serial: Option<Value>`; pass `Some(json!({"port": serial_port}))` in the running/final/handshake-failed reports (None for skipped/clone-failed).

- [x] **Step 3: Gate**

Run: `cargo fmt -p qol && cargo clippy -p qol --all-targets --all-features -- -D warnings && cargo test -p qol --all-features`
Expected: PASS

- [x] **Step 4: Commit**

```bash
git commit -m "feat(emu): expose guest serial console on a loopback socket"
```

### Task 2: SerialClient + Debian root shell + `qol emu sh`

**Files:**
- Create: `tools/qol-cli/src/commands/emu/serial.rs`
- Create: `tools/qol-cli/src/commands/emu/guest/mod.rs`, `tools/qol-cli/src/commands/emu/guest/debian.rs`
- Modify: `tools/qol-cli/src/commands/emu/live.rs` (serial_port in LiveRun)
- Modify: `tools/qol-cli/src/commands/emu/control.rs` (cmd_sh)
- Modify: `tools/qol-cli/src/commands/emu.rs` (mod decls, dispatch, help)

- [x] **Step 1: serial.rs with fake-server tests (mirror qmp.rs fake_server)**

```rust
pub(crate) struct SerialClient {
    stream: TcpStream,
    buffer: String,
}

pub(crate) fn connect(port: u16, timeout: Duration) -> Result<SerialClient>  // like qmp::connect, read timeout set

impl SerialClient {
    pub(crate) fn send_line(&mut self, line: &str) -> Result<()>  // writes line + "\n"

    pub(crate) fn wait_for(&mut self, marker: &str, timeout: Duration) -> Result<String> {
        // loop: if buffer contains marker, drain and return everything up to and including it;
        // else read socket chunk (WouldBlock/TimedOut => continue) until deadline, then bail
        // with "timed out waiting for `{marker}`; last output: {tail}"
    }

    pub(crate) fn run_command(&mut self, command: &str, timeout: Duration) -> Result<String> {
        self.send_line(&format!("{command}; echo QOL-\"RC\"-$?"))?;
        let output = self.wait_for("QOL-RC-", timeout)?;   // echoed command shows QOL-"RC"-$?, never QOL-RC-
        let rc = ...read digits after marker...;
        if rc != "0" { bail!("`{command}` exited {rc}; output: {output}") }
        Ok(output)
    }
}
```

Tests: fake TCP server scripting reads/replies; `wait_for` finds a marker split across two reads; `run_command` succeeds on `QOL-RC-0` and fails on `QOL-RC-1`; the echoed `QOL-"RC"-$?` does not satisfy the marker.

- [x] **Step 2: guest adapter skeleton**

`guest/mod.rs`:

```rust
mod debian;
pub(crate) use debian::DebianNocloud;

use super::serial::SerialClient;
use anyhow::Result;

pub(crate) trait GuestOs {
    fn ensure_root_shell(&self, serial: &mut SerialClient) -> Result<()>;
}
```

`guest/debian.rs`: `pub(crate) struct DebianNocloud;` whose `ensure_root_shell` sends a newline, waits briefly for either `login:` (then sends `root` and waits for `:~#`) or an existing `:~#` prompt.

- [x] **Step 3: `qol emu sh <id> <command...>`**

`live.rs`: `LiveRun` gains `serial_port: Option<u16>` parsed from `report["serial"]["port"]`. `control.rs`:

```rust
pub(crate) fn cmd_sh(args: &[OsString], verbose: bool) -> Result<()> {
    // id_and_rest, live::find, serial_port.ok_or("run has no serial console; rerun `qol emu up`"),
    // serial::connect, DebianNocloud.ensure_root_shell, run_command(joined, 30s), print output
}
```

Dispatch `"sh"`, help text line, SKILL update deferred to Task 6.

- [x] **Step 4: Gate + commit**

```bash
git commit -m "feat(emu): sh verb runs commands over the guest serial console"
```

### Task 3: Extract the boot pipeline from cmd_up

**Files:**
- Modify: `tools/qol-cli/src/commands/emu.rs`

- [x] **Step 1: Pure refactor, behavior identical**

Extract from `cmd_up`:

```rust
struct BootedVm {
    environment: Environment,
    resolution: Resolution,
    run_dir: PathBuf,
    qemu_command_path: PathBuf,
    commands: Vec<serde_json::Value>,
    qmp_port: u16,
    serial_port: u16,
    qemu_version: String,
    vm_status: String,
    child: std::process::Child,
    started_at: u64,
}

fn boot_vm(target: &str, command_name: &str, verbose: bool) -> Result<BootedVm>
// resolve -> run_dir -> skipped report path -> overlay clone -> spawn -> qmp handshake -> "running" report

fn finalize_vm(vm: BootedVm, exit: ExitStatus, workflow: Option<serde_json::Value>) -> Result<PathBuf>
// teardown -> final pass/failed report (workflow field when Some) -> returns report path
```

`ReportInput` gains `workflow: Option<serde_json::Value>` (rendered as a top-level `workflow` key when Some; `cmd_up` passes None everywhere). `cmd_up` becomes: parse args, `boot_vm`, wait on child, `finalize_vm`, step labels.

- [x] **Step 2: Gate (all 80+ tests must stay green, no behavior change) + commit**

```bash
git commit -m "refactor(emu): extract boot pipeline for workflow reuse"
```

### Task 4: Run facade, debian verbs, leaves-no-trace, `qol emu check`

**Files:**
- Modify: `tools/qol-cli/src/commands/emu/guest/mod.rs`, `guest/debian.rs`
- Create: `tools/qol-cli/src/commands/emu/workflow.rs`
- Modify: `tools/qol-cli/src/commands/emu.rs` (cmd_check, dispatch, help)

- [x] **Step 1: Grow the GuestOs trait**

```rust
pub(crate) trait GuestOs {
    fn ensure_root_shell(&self, serial: &mut SerialClient) -> Result<()>;
    fn launch_qol_from_stick(&self, serial: &mut SerialClient) -> Result<()>;
    fn reboot_and_relogin(&self, serial: &mut SerialClient) -> Result<()>;
    fn list_qol_traces(&self, serial: &mut SerialClient) -> Result<Vec<String>>;
}
```

Debian impl: `launch_qol_from_stick` runs over serial (stick = first USB SCSI disk, `/dev/sda`):

```text
mkfs.ext2 -q /dev/sda
mount /dev/sda /mnt
printf '%s\n' '#!/bin/sh' 'echo qol-stub start' 'mkdir -p /tmp/qol-stub' 'date > /tmp/qol-stub/scratch' 'echo qol-stub done' > /mnt/qol-stub.sh
sh /mnt/qol-stub.sh
umount /mnt
```

`reboot_and_relogin`: `send_line("reboot")`, `wait_for("login:", 180s)`, login. `list_qol_traces`: `run_command("find / -xdev -iname '*qol*' 2>/dev/null", 60s)` parsed by a pure `fn parse_traces(output: &str) -> Vec<String>`: keep lines starting with `/`, drop the echoed `find /` command line and marker lines. Unit-test `parse_traces` with a fixture containing echo, hits, and marker.

- [x] **Step 2: workflow.rs**

```rust
pub(crate) struct Verdict {
    pub(crate) pass: bool,
    pub(crate) traces: Vec<String>,
}

pub(crate) struct Run<'a> {
    pub(crate) qmp: &'a mut QmpClient,
    pub(crate) serial: &'a mut SerialClient,
    pub(crate) os: &'a dyn GuestOs,
    pub(crate) stick: &'a Path,
}

impl Run<'_> {
    fn insert / launch_qol / pull / reboot / list_traces  // thin: QMP attach/detach, adapter calls
}

pub(crate) fn leaves_no_trace(run: &mut Run) -> Result<Verdict> {
    run.insert()?;
    run.launch_qol()?;
    run.pull()?;
    run.reboot()?;
    let traces = run.list_traces()?;
    Ok(Verdict { pass: traces.is_empty(), traces })
}
```

- [x] **Step 3: cmd_check**

`qol emu check <id>`: `boot_vm(target, "check", verbose)` -> `qmp::connect` + `serial::connect` -> `ensure_root_shell` (boot wait: first `wait_for("login:", 180s)` happens inside it via the dual-marker logic; give the fresh-boot path the long timeout) -> `machine::ensure_usb_stick` -> run `leaves_no_trace` with step_labels per verb -> `qmp.fire("quit")` -> `child.wait()` -> `finalize_vm` with `workflow: Some(json!({"id": "leaves-no-trace", "verdict": if pass {"pass"} else {"fail"}, "traces": traces}))`. A failing workflow (Err or `pass: false`) still quits the VM and finalizes, then bails listing the traces. Dispatch `"check"`, help line.

- [x] **Step 4: Gate + commit**

```bash
git commit -m "feat(emu): leaves-no-trace workflow with debian guest adapter"
```

### Task 5: Runtime verification on the arm64 host

- [x] **Step 1: Image + config**

Download `https://cloud.debian.org/images/cloud/trixie/latest/debian-13-nocloud-arm64.qcow2` to `~/VMs/` and register it (platform config dir, macOS shown):

```toml
[images.debian-13-nocloud-arm64]
path = "~/VMs/debian-13-nocloud-arm64.qcow2"
arch = "aarch64"
```

`qol emu list` must show it `ready` from source `config`.

- [x] **Step 2: Clean run passes**

`cargo run -q -p qol -- emu check debian-13-nocloud-arm64`: expect hvf boot, root login over serial, stick insert/format/stub/pull, reboot, empty trace list, report `workflow.verdict == "pass"`, teardown removed overlay + stick.

Verified 2026-06-11:

- Clean report: `target/qol-emu/debian-13-nocloud-arm64-1781165400077/report.json`
- `workflow.verdict == "pass"`
- `workflow.traces == []`
- Teardown removed `overlay.qcow2` and `usb-stick.raw`

- [x] **Step 3: Dirty run fails (detection self-test, not committed)**

Temporarily add `'echo residue > /root/.qol-residue'` to the stub lines, rerun check, expect verdict `fail` with `/root/.qol-residue` listed. Revert the edit (`git checkout -- tools/qol-cli/src/commands/emu/guest/debian.rs`).

Verified 2026-06-11:

- Dirty report: `target/qol-emu/debian-13-nocloud-arm64-1781165639631/report.json`
- `workflow.verdict == "fail"`
- `workflow.traces == ["/root/.qol-residue"]`
- Temporary stub edit reverted

- [x] **Step 4: Keep the image + config** (the first real emu in the library); clean only `target/qol-emu` run dirs if oversized.

### Task 6: Docs

**Files:**
- Modify: `apps/qol-tray/skills/qol-cli-commands/SKILL.md` (sh/check verbs, M3 status, debian image setup, serial socket note)
- Modify: `docs/superpowers/specs/2026-06-10-emu-test-harness-design.md` (Status: M3 implemented + the four MVP deviations)

- [x] **Step 1: Update `qol-cli-commands` skill**
- [x] **Step 2: Update emu design spec status**

- [x] **Step 1: Update docs + commit plan file**

```bash
git commit -m "docs(emu): document m3 leaves-no-trace workflow"
```
