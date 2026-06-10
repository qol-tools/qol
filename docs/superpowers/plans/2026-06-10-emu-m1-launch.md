# emu M1: Launch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `qol emu up <id>` boots the prepared qcow2 overlay in QEMU with per-host acceleration, proves control by negotiating a QMP socket, and on VM exit discards the overlay so no-trace holds by construction.

**Architecture:** Extends the existing `tools/qol-cli/src/commands/emu.rs` MVP (which prepares an overlay + `qemu-command.txt` + `report.json`, then stops). Two new submodules: `machine.rs` (spawn, free-port probe, teardown) and `qmp.rs` (TCP QMP client: greeting, `qmp_capabilities`, `query-status`). QMP runs over TCP loopback, not a unix socket, so the same code compiles on all three hosts under `-D warnings`. Host `platform/` modules gain a `display()` verb and real accelerators (`hvf`, `whpx`).

**Tech Stack:** Rust (std only: `TcpStream`/`TcpListener`, `process::Command`), `serde_json`, `anyhow`. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-06-10-emu-test-harness-design.md` (Milestone M1).

**Out of scope (explicitly):** snapshot/screendump/sendkey/USB hot-plug (M2), `Machine` trait extraction (premature with one impl), Ctrl+C cleanup of an interrupted run (stale run dirs stay; note in report `next`), headless mode.

**Conventions that bind every task:** no code comments, conventional one-line commits without co-authors, exhaustive enum matches (no `_ =>`), table-driven tests with context in assertions, `cargo fmt --all --check && cargo clippy -p qol-cli --all-targets -- -D warnings && cargo test -p qol-cli` must pass before every commit.

---

## File Structure

- Modify: `tools/qol-cli/src/commands/emu/platform/linux.rs` (add `display()`)
- Modify: `tools/qol-cli/src/commands/emu/platform/macos.rs` (hvf, `display()`)
- Modify: `tools/qol-cli/src/commands/emu/platform/windows.rs` (whpx, `display()`)
- Create: `tools/qol-cli/src/commands/emu/qmp.rs` (QMP line classifier + TCP client)
- Create: `tools/qol-cli/src/commands/emu/machine.rs` (port probe, spawn, teardown)
- Modify: `tools/qol-cli/src/commands/emu.rs` (`qemu_args` signature, `cmd_up` launch flow, `ReportInput` fields, module decls)
- Modify: `apps/qol-tray/skills/qol-cli-commands/SKILL.md` (emu section: `up` now boots)

---

### Task 1: Per-host acceleration and display

`platform::acceleration()` returns `tcg` on macOS/Windows today; QEMU there should use the native hypervisor. Workflows render the VM via a per-host display backend. Constants only, so no tests (thin wrappers).

**Files:**
- Modify: `tools/qol-cli/src/commands/emu/platform/linux.rs`
- Modify: `tools/qol-cli/src/commands/emu/platform/macos.rs`
- Modify: `tools/qol-cli/src/commands/emu/platform/windows.rs`

- [ ] **Step 1: Add `display()` to linux.rs**

Append to `tools/qol-cli/src/commands/emu/platform/linux.rs`:

```rust
pub(crate) fn display() -> &'static str {
    "gtk"
}
```

- [ ] **Step 2: Fix macos.rs acceleration and add display**

Replace the `acceleration` function in `tools/qol-cli/src/commands/emu/platform/macos.rs` and append `display`:

```rust
pub(crate) fn acceleration() -> &'static str {
    "hvf"
}

pub(crate) fn display() -> &'static str {
    "cocoa"
}
```

- [ ] **Step 3: Fix windows.rs acceleration and add display**

Replace the `acceleration` function in `tools/qol-cli/src/commands/emu/platform/windows.rs` and append `display`:

```rust
pub(crate) fn acceleration() -> &'static str {
    "whpx"
}

pub(crate) fn display() -> &'static str {
    "sdl"
}
```

- [ ] **Step 4: Build**

`display()` is not consumed yet; the current OS's platform module will trip `dead_code` under clippy `-D warnings`. That is expected mid-plan: run only `cargo build -p qol-cli` here (warnings allowed), and rely on Task 2 wiring `display()` into `qemu_args` before the next clippy gate. Do NOT add `#[allow(dead_code)]`.

Run: `cargo build -p qol-cli`
Expected: success (a `dead_code` warning for `display` is acceptable at this step only).

- [ ] **Step 5: Do not commit yet**

This task commits together with Task 2, so the tree never holds a committed dead symbol (cross-platform hygiene rule).

### Task 2: `qemu_args` gains display + QMP wiring

**Files:**
- Modify: `tools/qol-cli/src/commands/emu.rs` (function `qemu_args` around line 420, tests module at bottom)

- [ ] **Step 1: Write the failing test**

Add to the `tests` module at the bottom of `tools/qol-cli/src/commands/emu.rs`:

```rust
#[test]
fn qemu_args_wire_accel_display_and_qmp() {
    let environment = Environment {
        id: "foo".to_string(),
        name: "Foo".to_string(),
        backend: "qemu".to_string(),
        arch: "x86_64".to_string(),
        image_path: PathBuf::from("/a/b/base.qcow2"),
        source: "config".to_string(),
    };
    let args = qemu_args(
        &environment,
        Path::new("/a/b/overlay.qcow2"),
        "kvm",
        "gtk",
        4444,
    );
    let joined = args.join(" ");
    let expected = [
        "-accel kvm",
        "-display gtk",
        "-qmp tcp:127.0.0.1:4444,server,nowait",
        "-drive file=/a/b/overlay.qcow2,if=virtio,format=qcow2",
    ];
    for fragment in expected {
        assert!(joined.contains(fragment), "missing `{fragment}` in: {joined}");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p qol-cli qemu_args_wire -- --nocapture`
Expected: FAIL to compile ("this function takes 3 arguments but 5 arguments were supplied").

- [ ] **Step 3: Extend `qemu_args`**

Replace the existing `qemu_args` function in `tools/qol-cli/src/commands/emu.rs`:

```rust
fn qemu_args(
    environment: &Environment,
    overlay: &Path,
    acceleration: &str,
    display: &str,
    qmp_port: u16,
) -> Vec<String> {
    vec![
        "-name".to_string(),
        format!("qol-emu-{}", environment.id),
        "-machine".to_string(),
        "q35".to_string(),
        "-accel".to_string(),
        acceleration.to_string(),
        "-m".to_string(),
        "4096".to_string(),
        "-smp".to_string(),
        "2".to_string(),
        "-drive".to_string(),
        format!("file={},if=virtio,format=qcow2", overlay.display()),
        "-nic".to_string(),
        "user,model=virtio-net-pci".to_string(),
        "-display".to_string(),
        display.to_string(),
        "-qmp".to_string(),
        format!("tcp:127.0.0.1:{qmp_port},server,nowait"),
    ]
}
```

Update the single call site in `cmd_up` (full `cmd_up` rewrite lands in Task 6; here just keep it compiling):

```rust
    let qemu_args = qemu_args(&environment, &overlay, resolution.acceleration, platform::display(), 4444);
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p qol-cli`
Expected: PASS (all existing emu tests plus the new one).

- [ ] **Step 5: Gate and commit Tasks 1+2**

Run: `cargo fmt --all && cargo fmt --all --check && cargo clippy -p qol-cli --all-targets -- -D warnings && cargo test -p qol-cli`
Expected: clean.

```bash
git add tools/qol-cli/src/commands/emu.rs tools/qol-cli/src/commands/emu/platform/
git commit -m "feat(emu): wire per-host accel, display, and qmp args"
```

(The hardcoded `4444` placeholder port is removed in Task 6 when the free-port probe lands.)

### Task 3: QMP line classifier (pure parsing)

QMP is line-delimited JSON: a greeting on connect, then `return` / `error` responses interleaved with async `event` lines. Classify each line into an exhaustive enum so the client loop never guesses.

**Files:**
- Create: `tools/qol-cli/src/commands/emu/qmp.rs`
- Modify: `tools/qol-cli/src/commands/emu.rs` (add `mod qmp;` next to `mod discovery;`)

- [ ] **Step 1: Create the module with failing tests**

Create `tools/qol-cli/src/commands/emu/qmp.rs`:

```rust
use anyhow::{bail, Context, Result};
use serde_json::Value;

#[derive(Debug)]
pub(crate) enum QmpLine {
    Greeting { qemu_version: String },
    Return(Value),
    Event(String),
    Error(String),
}

pub(crate) fn classify_line(line: &str) -> Result<QmpLine> {
    let value: Value = serde_json::from_str(line)
        .with_context(|| format!("qmp line is not JSON: {line}"))?;
    if value.get("QMP").is_some() {
        return Ok(QmpLine::Greeting {
            qemu_version: greeting_version(&value),
        });
    }
    if let Some(event) = value.get("event").and_then(Value::as_str) {
        return Ok(QmpLine::Event(event.to_string()));
    }
    if let Some(error) = value.get("error") {
        return Ok(QmpLine::Error(error.to_string()));
    }
    if let Some(result) = value.get("return") {
        return Ok(QmpLine::Return(result.clone()));
    }
    bail!("unrecognized qmp line: {line}")
}

fn greeting_version(value: &Value) -> String {
    let qemu = value
        .get("QMP")
        .and_then(|qmp| qmp.get("version"))
        .and_then(|version| version.get("qemu"));
    let part = |key: &str| {
        qemu.and_then(|qemu| qemu.get(key))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    format!("{}.{}.{}", part("major"), part("minor"), part("micro"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_qmp_lines() {
        let cases = [
            (
                r#"{"QMP":{"version":{"qemu":{"major":9,"minor":2,"micro":1}},"capabilities":[]}}"#,
                "greeting 9.2.1",
            ),
            (r#"{"return":{}}"#, "return"),
            (
                r#"{"event":"POWERDOWN","timestamp":{"seconds":0,"microseconds":0}}"#,
                "event POWERDOWN",
            ),
            (
                r#"{"error":{"class":"GenericError","desc":"nope"}}"#,
                "error",
            ),
        ];
        for (line, expected) in cases {
            let label = match classify_line(line).unwrap() {
                QmpLine::Greeting { qemu_version } => format!("greeting {qemu_version}"),
                QmpLine::Return(_) => "return".to_string(),
                QmpLine::Event(name) => format!("event {name}"),
                QmpLine::Error(_) => "error".to_string(),
            };
            assert_eq!(label, expected, "line: {line}");
        }
    }

    #[test]
    fn rejects_garbage_lines() {
        for line in ["not json", r#"{"unrelated":1}"#] {
            assert!(classify_line(line).is_err(), "should reject: {line}");
        }
    }
}
```

And in `tools/qol-cli/src/commands/emu.rs`, next to `mod discovery;` / `mod platform;`:

```rust
mod qmp;
```

- [ ] **Step 2: Run tests to verify they pass**

(The classifier and tests land together since the file is new; the failing state was its absence.)

Run: `cargo test -p qol-cli qmp -- --nocapture`
Expected: PASS, 2 tests.

- [ ] **Step 3: Commit**

```bash
git add tools/qol-cli/src/commands/emu/qmp.rs tools/qol-cli/src/commands/emu.rs
git commit -m "feat(emu): add qmp line classifier"
```

(`clippy -D warnings` will flag `classify_line` as dead until Task 4 consumes it; if it does, fold this commit into Task 4's instead of allowing the lint.)

### Task 4: QMP TCP client (connect, negotiate, query-status)

**Files:**
- Modify: `tools/qol-cli/src/commands/emu/qmp.rs`

- [ ] **Step 1: Write the failing test (fake QMP server on a loopback listener)**

Add to the `tests` module in `qmp.rs`:

```rust
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    #[test]
    fn connects_negotiates_and_queries_status() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut stream = stream;
            writeln!(
                stream,
                r#"{{"QMP":{{"version":{{"qemu":{{"major":9,"minor":2,"micro":0}}}},"capabilities":[]}}}}"#
            )
            .unwrap();
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(line.contains("qmp_capabilities"), "first command: {line}");
            writeln!(stream, r#"{{"return":{{}}}}"#).unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            assert!(line.contains("query-status"), "second command: {line}");
            writeln!(
                stream,
                r#"{{"event":"NIC_RX_FILTER_CHANGED","timestamp":{{"seconds":0,"microseconds":0}}}}"#
            )
            .unwrap();
            writeln!(stream, r#"{{"return":{{"status":"running","running":true}}}}"#).unwrap();
        });
        let mut client = connect(port, Duration::from_secs(2)).unwrap();
        assert_eq!(client.qemu_version, "9.2.0");
        assert_eq!(client.query_status().unwrap(), "running");
        server.join().unwrap();
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p qol-cli connects_negotiates -- --nocapture`
Expected: FAIL to compile ("cannot find function `connect`").

- [ ] **Step 3: Implement the client**

Add above the tests in `qmp.rs` (extend the existing `use` lines accordingly):

```rust
use anyhow::anyhow;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub(crate) struct QmpClient {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    pub(crate) qemu_version: String,
}

pub(crate) fn connect(port: u16, timeout: Duration) -> Result<QmpClient> {
    let deadline = Instant::now() + timeout;
    let address = format!("127.0.0.1:{port}");
    loop {
        match TcpStream::connect(&address) {
            Ok(stream) => return QmpClient::handshake(stream),
            Err(error) => {
                if Instant::now() >= deadline {
                    return Err(error)
                        .with_context(|| format!("qmp connect to {address} timed out"));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

impl QmpClient {
    fn handshake(stream: TcpStream) -> Result<Self> {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .context("failed to set qmp read timeout")?;
        let reader = BufReader::new(stream.try_clone().context("failed to clone qmp stream")?);
        let mut client = Self {
            stream,
            reader,
            qemu_version: String::new(),
        };
        let line = client.read_line()?;
        match classify_line(&line)? {
            QmpLine::Greeting { qemu_version } => client.qemu_version = qemu_version,
            QmpLine::Return(_) | QmpLine::Event(_) | QmpLine::Error(_) => {
                bail!("expected qmp greeting, got: {line}")
            }
        }
        client.execute("qmp_capabilities")?;
        Ok(client)
    }

    fn execute(&mut self, command: &str) -> Result<Value> {
        let request = serde_json::json!({ "execute": command });
        writeln!(self.stream, "{request}")
            .with_context(|| format!("failed to send qmp command {command}"))?;
        loop {
            let line = self.read_line()?;
            match classify_line(&line)? {
                QmpLine::Return(value) => return Ok(value),
                QmpLine::Event(_) => continue,
                QmpLine::Greeting { .. } => bail!("unexpected qmp greeting mid-session"),
                QmpLine::Error(error) => bail!("qmp {command} failed: {error}"),
            }
        }
    }

    pub(crate) fn query_status(&mut self) -> Result<String> {
        let value = self.execute("query-status")?;
        value
            .get("status")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| anyhow!("query-status returned no status: {value}"))
    }

    fn read_line(&mut self) -> Result<String> {
        let mut line = String::new();
        let bytes = self
            .reader
            .read_line(&mut line)
            .context("failed to read qmp line")?;
        if bytes == 0 {
            bail!("qmp connection closed");
        }
        Ok(line)
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p qol-cli qmp -- --nocapture`
Expected: PASS, 3 tests (classifier x2 + client).

- [ ] **Step 5: Gate and commit**

Run: `cargo fmt --all && cargo fmt --all --check && cargo clippy -p qol-cli --all-targets -- -D warnings && cargo test -p qol-cli`
Expected: clean (clippy may still flag `connect`/`query_status` as dead until Task 6 consumes them; if so, defer this commit and fold it into Task 6's commit rather than allowing the lint).

```bash
git add tools/qol-cli/src/commands/emu/qmp.rs
git commit -m "feat(emu): add qmp tcp client with capabilities negotiation"
```

### Task 5: machine.rs (free port, spawn, teardown)

**Files:**
- Create: `tools/qol-cli/src/commands/emu/machine.rs`
- Modify: `tools/qol-cli/src/commands/emu.rs` (add `mod machine;`)

- [ ] **Step 1: Create the module with tests**

Create `tools/qol-cli/src/commands/emu/machine.rs`:

```rust
use anyhow::{Context, Result};
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const DISPOSABLE_FILES: [&str; 1] = ["overlay.qcow2"];

pub(crate) fn free_qmp_port() -> Result<u16> {
    let listener =
        TcpListener::bind("127.0.0.1:0").context("failed to probe a free qmp port")?;
    Ok(listener
        .local_addr()
        .context("failed to read qmp probe address")?
        .port())
}

pub(crate) fn spawn_qemu(qemu_system: &Path, args: &[String]) -> Result<Child> {
    Command::new(qemu_system)
        .args(args)
        .stdin(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to spawn {}", qemu_system.display()))
}

pub(crate) fn teardown(run_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    for name in DISPOSABLE_FILES {
        let path = run_dir.join(name);
        if path.is_file() {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
            removed.push(path);
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_qmp_port_returns_bindable_port() {
        let port = free_qmp_port().unwrap();
        assert_ne!(port, 0);
    }

    #[test]
    fn teardown_removes_overlay_and_keeps_artifacts() {
        let dir = std::env::temp_dir().join(format!("qol-emu-teardown-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("overlay.qcow2"), b"x").unwrap();
        fs::write(dir.join("report.json"), b"{}").unwrap();
        fs::write(dir.join("qemu-command.txt"), b"qemu").unwrap();
        let removed = teardown(&dir).unwrap();
        assert_eq!(removed, vec![dir.join("overlay.qcow2")]);
        let expectations = [
            ("overlay.qcow2", false),
            ("report.json", true),
            ("qemu-command.txt", true),
        ];
        for (name, should_exist) in expectations {
            assert_eq!(dir.join(name).exists(), should_exist, "file: {name}");
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
```

And in `tools/qol-cli/src/commands/emu.rs`, next to the other module declarations:

```rust
mod machine;
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p qol-cli machine -- --nocapture`
Expected: PASS, 2 tests.

- [ ] **Step 3: Commit**

```bash
git add tools/qol-cli/src/commands/emu/machine.rs tools/qol-cli/src/commands/emu.rs
git commit -m "feat(emu): add machine spawn, port probe, and teardown"
```

(Same dead-code caveat as Task 4: fold into Task 6's commit if clippy objects.)

### Task 6: Wire launch into `cmd_up`

The flow becomes: resolve, overlay, write `qemu-command.txt`, boot QEMU, confirm over QMP, write a `running` report, wait for VM exit, teardown overlay, write the final report.

**Files:**
- Modify: `tools/qol-cli/src/commands/emu.rs` (`cmd_up`, `ReportInput`, `report_json`, imports)

- [ ] **Step 1: Extend `ReportInput` and `report_json`**

In `tools/qol-cli/src/commands/emu.rs`, add two fields to `ReportInput`:

```rust
struct ReportInput<'a> {
    environment: &'a Environment,
    resolution: &'a Resolution,
    run_dir: &'a Path,
    status: &'a str,
    overlay: Option<&'a Path>,
    qemu_command: Option<&'a Path>,
    commands: Vec<serde_json::Value>,
    qmp: Option<serde_json::Value>,
    teardown: Option<serde_json::Value>,
    next: Vec<String>,
    started_at: u64,
}
```

In `report_json`, add the two keys after `"commands"`:

```rust
        "commands": input.commands,
        "qmp": input.qmp,
        "teardown": input.teardown,
        "next": input.next,
```

The two existing `report_json(ReportInput { ... })` call sites (the `skipped` branch and the qemu-img `failed` branch) each gain:

```rust
            qmp: None,
            teardown: None,
```

- [ ] **Step 2: Replace the tail of `cmd_up`**

Add `Duration` to the existing `std::time` import at the top of `emu.rs`:

```rust
use std::time::{Duration, SystemTime, UNIX_EPOCH};
```

Then replace everything in `cmd_up` from `let qemu_system = resolution...` down to the final `Ok(())` with:

```rust
    let qemu_system = resolution
        .qemu_system
        .clone()
        .ok_or_else(|| anyhow!("ready environment has no qemu-system path"))?;
    let qmp_port = machine::free_qmp_port()?;
    let qemu_args = qemu_args(
        &environment,
        &overlay,
        resolution.acceleration,
        platform::display(),
        qmp_port,
    );
    let qemu_command = command_line(&qemu_system, &qemu_args);
    let qemu_command_path = run_dir.join("qemu-command.txt");
    fs::write(&qemu_command_path, format!("{qemu_command}\n"))
        .with_context(|| format!("failed to write {}", qemu_command_path.display()))?;
    let commands = vec![
        json!({
            "program": qemu_img,
            "args": ["info", "--output=json", &resolution.image_path.display().to_string()],
            "detected_format": image_format,
        }),
        json!({
            "program": qemu_img,
            "args": create_args,
            "status": status.to_string(),
        }),
        json!({
            "program": qemu_system,
            "args": qemu_args,
        }),
    ];

    step_label("boot", StepKind::Pending, &format!("{} · qmp 127.0.0.1:{qmp_port}", environment.id));
    let mut child = machine::spawn_qemu(&qemu_system, &qemu_args)?;
    let handshake = qmp::connect(qmp_port, Duration::from_secs(10))
        .and_then(|mut client| {
            let status = client.query_status()?;
            Ok((client.qemu_version.clone(), status))
        });
    let (qemu_version, vm_status) = match handshake {
        Ok(values) => values,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            let removed = machine::teardown(&run_dir)?;
            let report = report_json(ReportInput {
                environment: &environment,
                resolution: &resolution,
                run_dir: &run_dir,
                status: "failed",
                overlay: Some(&overlay),
                qemu_command: Some(&qemu_command_path),
                commands,
                qmp: Some(json!({ "port": qmp_port, "error": error.to_string() })),
                teardown: Some(json!({ "removed": removed })),
                next: vec!["Inspect the qemu output above, then rerun `qol emu up`.".to_string()],
                started_at,
            })?;
            write_report(&run_dir, &report)?;
            bail!("qmp handshake failed: {error:#}");
        }
    };
    step_label("qmp", StepKind::Success, &format!("qemu {qemu_version} · {vm_status}"));
    let report = report_json(ReportInput {
        environment: &environment,
        resolution: &resolution,
        run_dir: &run_dir,
        status: "running",
        overlay: Some(&overlay),
        qemu_command: Some(&qemu_command_path),
        commands: commands.clone(),
        qmp: Some(json!({ "port": qmp_port, "qemu_version": qemu_version, "status": vm_status })),
        teardown: None,
        next: vec!["Close the VM window (or shut the guest down) to end the run.".to_string()],
        started_at,
    })?;
    write_report(&run_dir, &report)?;
    step_label("running", StepKind::Success, "close the VM window to end the run");

    let exit = child.wait().context("failed to wait for qemu")?;
    let removed = machine::teardown(&run_dir)?;
    let final_status = if exit.success() { "pass" } else { "failed" };
    let report = report_json(ReportInput {
        environment: &environment,
        resolution: &resolution,
        run_dir: &run_dir,
        status: final_status,
        overlay: None,
        qemu_command: Some(&qemu_command_path),
        commands,
        qmp: Some(json!({ "port": qmp_port, "qemu_version": qemu_version, "status": vm_status })),
        teardown: Some(json!({ "removed": removed, "exit": exit.to_string() })),
        next: vec!["Rerun `qol emu up` for a fresh disposable clone.".to_string()],
        started_at,
    })?;
    write_report(&run_dir, &report)?;
    step_label(
        "clean",
        StepKind::Success,
        &format!("removed {} disposable file(s)", removed.len()),
    );
    step_label(
        "report",
        StepKind::Info,
        &run_dir.join("report.json").display().to_string(),
    );
    if !exit.success() {
        bail!("qemu exited with {exit}");
    }
    Ok(())
```

Note: the old `"pass"`-on-prepare report and its `next: Run {qemu_command}` text are gone; `status: "pass"` now means the VM booted, ran, exited cleanly, and the overlay was discarded.

- [ ] **Step 3: Gate**

Run: `cargo fmt --all && cargo fmt --all --check && cargo clippy -p qol-cli --all-targets -- -D warnings && cargo test -p qol-cli`
Expected: clean. This is also the step where any deferred Task 4/5 dead-code holds resolve.

- [ ] **Step 4: Manual verification (machine-dependent)**

Needs a host with QEMU installed and at least one `ready` emu (`qol emu list`). On the Mac dev box: `brew install qemu` plus an `[images]` entry in `~/.config/qol-tray/emu.toml`; otherwise run on the Linux box.

```bash
cargo run -p qol-cli -- emu list
cargo run -p qol-cli -- emu up <ready-id>
```

Expected: `boot` label with the QMP port, a VM window opens, `qmp` label shows the QEMU version and `running`, `running` label appears. Close the VM window. Expected: `clean` label reports 1 removed file, `report` label prints the path; the run dir holds `report.json` (status `pass`, `teardown.removed` lists the overlay) and `qemu-command.txt`, and `overlay.qcow2` is gone.

If no QEMU host is available, state that runtime verification is pending; do not claim it.

- [ ] **Step 5: Commit**

```bash
git add tools/qol-cli/src/commands/emu.rs tools/qol-cli/src/commands/emu/qmp.rs tools/qol-cli/src/commands/emu/machine.rs
git commit -m "feat(emu): boot the overlay via qemu with qmp confirmation and teardown"
```

### Task 7: Documentation

**Files:**
- Modify: `apps/qol-tray/skills/qol-cli-commands/SKILL.md`

- [ ] **Step 1: Update the emu section**

In the `## Emu` section, replace:

```markdown
- `qol emu up <id>`: create a disposable qcow2 overlay and write `qemu-command.txt` plus `report.json`; it does not launch the VM yet.
```

with:

```markdown
- `qol emu up <id>`: create a disposable qcow2 overlay, boot it in QEMU (per-host accel: kvm/hvf/whpx), confirm control over a loopback QMP socket, and block until the VM exits; teardown removes the overlay and leaves `report.json` + `qemu-command.txt` in the run directory. Report statuses: `running` while up, then `pass` / `failed` / `skipped`.
```

- [ ] **Step 2: Commit**

```bash
git add apps/qol-tray/skills/qol-cli-commands/SKILL.md
git commit -m "docs(emu): document launching qol emu up"
```

- [ ] **Step 3: Reinstall the CLI**

Run: `qol setup`
Expected: the installed `qol` binary now carries the new `up`; `qol emu doctor` still works.
