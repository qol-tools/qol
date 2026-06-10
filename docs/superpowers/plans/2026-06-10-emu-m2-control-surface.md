# emu M2: QMP Control Surface Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Drive a running `qol emu up` VM from a second process: screenshot, send keys, attach/detach a USB stick, take a disk snapshot, and shut it down - all over the existing QMP loopback socket.

**Architecture:** `report.json` (status `running`, `qmp.port`) doubles as the live-run registry; new `qol emu <verb> <id>` subcommands resolve the newest running report for the id, connect to its QMP port, and issue one command. `QmpClient::execute` grows an `arguments` payload and `QmpLine::Event` regains its name so `DEVICE_DELETED` can be awaited. These verbs are the `Machine` half of `insert`/`pull`/`use` from the design spec; M3's `Run` facade will call the same functions programmatically.

**Tech Stack:** Rust (std TcpStream, serde_json, anyhow), QEMU QMP (`screendump`, `send-key`, `blockdev-add`/`device_add`, `blockdev-snapshot-sync`, `quit`).

**Spec:** `docs/superpowers/specs/2026-06-10-emu-test-harness-design.md` (M2 milestone).

**Out of scope (follow-up plan):** "Finish hvf and whpx" - arch-aware QEMU binary/accel selection and aarch64 guest wiring is an independent subsystem and ships as its own plan. No real guest OS is needed here; every verb works against a SeaBIOS screen.

**Conventions that bind every task:** no code comments; conventional one-line commits, no AI attribution; exhaustive enum matches (no `_ =>` on our enums); table-driven tests with context in assertions; gate = `cargo fmt -p qol && cargo clippy -p qol --all-targets --all-features -- -D warnings && cargo test -p qol --all-features`. The package is named `qol`, not `qol-cli`.

---

## File structure

- `tools/qol-cli/src/commands/emu/qmp.rs` (modify): `execute(command, arguments)`, `Event(String)`, `fire`, `wait_event`, verb methods (`screendump`, `send_keys`, `attach_usb_stick`, `detach_usb_stick`, `disk_snapshot`).
- `tools/qol-cli/src/commands/emu/live.rs` (create): resolve the newest running run for an id from `target/qol-emu/*/report.json`.
- `tools/qol-cli/src/commands/emu/control.rs` (create): the six `cmd_*` handlers (shot, key, down, insert, pull, snap) - thin orchestration only.
- `tools/qol-cli/src/commands/emu/machine.rs` (modify): `ensure_usb_stick`, teardown removes every VM disk image instead of a hardcoded list.
- `tools/qol-cli/src/commands/emu/discovery/mod.rs` (modify): re-export `is_vm_image_path` for teardown reuse.
- `tools/qol-cli/src/commands/emu.rs` (modify): dispatch new subcommands, `-device qemu-xhci` + `id=qoldisk` in `qemu_args`, help text, `pub(crate)` on `unix_millis`.
- `apps/qol-tray/skills/qol-cli-commands/SKILL.md` (modify, final task): document the verbs.

---

### Task 1: QMP arguments, named events, fire, wait_event

**Files:**
- Modify: `tools/qol-cli/src/commands/emu/qmp.rs`

- [ ] **Step 1: Write the failing tests**

In `qmp.rs` tests: change the classifier table's event expectation to carry the name, and add two fake-server tests.

```rust
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
fn execute_sends_arguments_payload() {
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
        writeln!(stream, r#"{{"return":{{}}}}"#).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert!(line.contains(r#""execute":"screendump""#), "line: {line}");
        assert!(line.contains(r#""filename":"/a/b/shot.ppm""#), "line: {line}");
        writeln!(stream, r#"{{"return":{{}}}}"#).unwrap();
    });
    let mut client = connect(port, Duration::from_secs(2)).unwrap();
    client
        .execute(
            "screendump",
            Some(serde_json::json!({"filename": "/a/b/shot.ppm"})),
        )
        .unwrap();
    server.join().unwrap();
}

#[test]
fn wait_event_skips_unrelated_lines_until_match() {
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
        writeln!(stream, r#"{{"return":{{}}}}"#).unwrap();
        writeln!(
            stream,
            r#"{{"event":"NIC_RX_FILTER_CHANGED","timestamp":{{"seconds":0,"microseconds":0}}}}"#
        )
        .unwrap();
        writeln!(
            stream,
            r#"{{"event":"DEVICE_DELETED","data":{{"device":"qolusbdev"}},"timestamp":{{"seconds":0,"microseconds":0}}}}"#
        )
        .unwrap();
    });
    let mut client = connect(port, Duration::from_secs(2)).unwrap();
    client
        .wait_event("DEVICE_DELETED", Duration::from_secs(2))
        .unwrap();
    server.join().unwrap();
}
```

The existing `connects_negotiates_and_queries_status` test stays as-is.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p qol qmp -- --nocapture`
Expected: compile errors (`Event` takes no payload, `execute` takes no arguments, no `wait_event`).

- [ ] **Step 3: Implement**

In `qmp.rs`:

```rust
#[derive(Debug)]
pub(crate) enum QmpLine {
    Greeting { qemu_version: String },
    Return(Value),
    Event(String),
    Error(String),
}
```

In `classify_line`, replace the event arm:

```rust
    if let Some(event) = value.get("event") {
        return Ok(QmpLine::Event(
            event.as_str().unwrap_or_default().to_string(),
        ));
    }
```

Replace `execute` and add `fire` and `wait_event`; update the two internal callers:

```rust
    pub(crate) fn execute(&mut self, command: &str, arguments: Option<Value>) -> Result<Value> {
        let mut request = serde_json::json!({ "execute": command });
        if let Some(arguments) = arguments {
            request["arguments"] = arguments;
        }
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

    pub(crate) fn fire(&mut self, command: &str) -> Result<()> {
        let request = serde_json::json!({ "execute": command });
        writeln!(self.stream, "{request}")
            .with_context(|| format!("failed to send qmp command {command}"))
    }

    pub(crate) fn wait_event(&mut self, name: &str, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let line = self.read_line()?;
            match classify_line(&line)? {
                QmpLine::Event(event) if event == name => return Ok(()),
                QmpLine::Event(_) | QmpLine::Return(_) => continue,
                QmpLine::Greeting { .. } => bail!("unexpected qmp greeting mid-session"),
                QmpLine::Error(error) => {
                    bail!("qmp error while waiting for event {name}: {error}")
                }
            }
        }
        bail!("timed out waiting for qmp event {name}")
    }
```

Handshake caller becomes `client.execute("qmp_capabilities", None)?;` and `query_status` becomes `self.execute("query-status", None)?`. `execute` must become `pub(crate)` since `control.rs` will call it later. Note: `read_line` already carries a 5s socket read timeout, so `wait_event` cannot hang forever even if the deadline math never fires.

- [ ] **Step 4: Run the gate**

Run: `cargo fmt -p qol && cargo clippy -p qol --all-targets --all-features -- -D warnings && cargo test -p qol --all-features`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add tools/qol-cli/src/commands/emu/qmp.rs
git commit -m "feat(emu): qmp arguments, named events, fire and wait_event"
```

---

### Task 2: Live-run resolution from report.json

**Files:**
- Create: `tools/qol-cli/src/commands/emu/live.rs`
- Modify: `tools/qol-cli/src/commands/emu.rs` (add `mod live;`)

- [ ] **Step 1: Write the failing tests**

`live.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    #[test]
    fn running_report_filters_id_status_and_port() {
        let running = json!({
            "environment": {"id": "foo"},
            "status": "running",
            "started_at_unix_ms": 10u64,
            "qmp": {"port": 4444},
        });
        let finished = json!({
            "environment": {"id": "foo"},
            "status": "pass",
            "started_at_unix_ms": 20u64,
            "qmp": {"port": 4445},
        });
        let other_id = json!({
            "environment": {"id": "bar"},
            "status": "running",
            "started_at_unix_ms": 30u64,
            "qmp": {"port": 4446},
        });
        let cases = [
            (&running, Some((10u64, 4444u16))),
            (&finished, None),
            (&other_id, None),
        ];
        for (report, expected) in cases {
            assert_eq!(running_report(report, "foo"), expected, "report: {report}");
        }
    }

    #[test]
    fn find_picks_newest_running_run() {
        let root = std::env::temp_dir().join(format!("qol-emu-live-{}", std::process::id()));
        let write = |dir: &str, report: serde_json::Value| {
            let run_dir = root.join(dir);
            fs::create_dir_all(&run_dir).unwrap();
            fs::write(run_dir.join("report.json"), report.to_string()).unwrap();
        };
        write(
            "foo-10",
            json!({"environment": {"id": "foo"}, "status": "running",
                   "started_at_unix_ms": 10u64, "qmp": {"port": 4444}}),
        );
        write(
            "foo-20",
            json!({"environment": {"id": "foo"}, "status": "running",
                   "started_at_unix_ms": 20u64, "qmp": {"port": 5555}}),
        );
        let live = find(&root, "foo").unwrap();
        assert_eq!(live.qmp_port, 5555);
        assert_eq!(live.run_dir, root.join("foo-20"));
        assert!(find(&root, "bar").is_err(), "bar has no running run");
        fs::remove_dir_all(&root).unwrap();
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p qol live`
Expected: compile error, module functions missing.

- [ ] **Step 3: Implement**

```rust
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) struct LiveRun {
    pub(crate) run_dir: PathBuf,
    pub(crate) qmp_port: u16,
}

pub(crate) fn find(runs_root: &Path, id: &str) -> Result<LiveRun> {
    let entries = fs::read_dir(runs_root).map_err(|_| no_live_run(id))?;
    let mut best: Option<(u64, LiveRun)> = None;
    for entry in entries.flatten() {
        let run_dir = entry.path();
        let Ok(content) = fs::read_to_string(run_dir.join("report.json")) else {
            continue;
        };
        let Ok(report) = serde_json::from_str::<Value>(&content) else {
            continue;
        };
        let Some((started_at, qmp_port)) = running_report(&report, id) else {
            continue;
        };
        let newer = best
            .as_ref()
            .is_none_or(|(best_started, _)| started_at > *best_started);
        if newer {
            best = Some((started_at, LiveRun { run_dir, qmp_port }));
        }
    }
    best.map(|(_, live)| live).ok_or_else(|| no_live_run(id))
}

fn no_live_run(id: &str) -> anyhow::Error {
    anyhow!("no running emu `{id}`; start one with `qol emu up {id}`")
}

fn running_report(report: &Value, id: &str) -> Option<(u64, u16)> {
    if report.get("environment")?.get("id")?.as_str()? != id {
        return None;
    }
    if report.get("status")?.as_str()? != "running" {
        return None;
    }
    let started_at = report.get("started_at_unix_ms")?.as_u64()?;
    let port = u16::try_from(report.get("qmp")?.get("port")?.as_u64()?).ok()?;
    Some((started_at, port))
}
```

Add `mod live;` next to `mod machine;` in `emu.rs`. Accepted MVP behavior: a run killed with SIGKILL leaves a stale `running` report; the verb then fails at `qmp::connect` with a timeout naming the port, which is honest enough.

- [ ] **Step 4: Run the gate** (same command as Task 1). Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add tools/qol-cli/src/commands/emu/live.rs tools/qol-cli/src/commands/emu.rs
git commit -m "feat(emu): resolve live runs from running report.json entries"
```

---

### Task 3: USB controller and named disk in qemu_args

**Files:**
- Modify: `tools/qol-cli/src/commands/emu.rs` (`qemu_args` and its test)

- [ ] **Step 1: Extend the failing test**

In `qemu_args_wire_accel_display_and_qmp`, extend `expected`:

```rust
        let expected = [
            "-accel kvm",
            "-display gtk",
            "-qmp tcp:127.0.0.1:4444,server,nowait",
            "-drive file=/a/b/overlay.qcow2,id=qoldisk,if=virtio,format=qcow2",
            "-device qemu-xhci,id=xhci",
        ];
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p qol qemu_args`
Expected: FAIL, missing fragments.

- [ ] **Step 3: Implement**

In `qemu_args`, change the drive line and add the controller after the `-nic` pair:

```rust
        "-drive".to_string(),
        format!("file={},id=qoldisk,if=virtio,format=qcow2", overlay.display()),
        "-nic".to_string(),
        "user,model=virtio-net-pci".to_string(),
        "-device".to_string(),
        "qemu-xhci,id=xhci".to_string(),
```

`qoldisk` is the stable device name `blockdev-snapshot-sync` targets in Task 6; `xhci` is the hot-plug bus for Task 5.

- [ ] **Step 4: Run the gate.** Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add tools/qol-cli/src/commands/emu.rs
git commit -m "feat(emu): add xhci controller and named disk to qemu launch args"
```

---

### Task 4: shot, key, and down commands

**Files:**
- Create: `tools/qol-cli/src/commands/emu/control.rs`
- Modify: `tools/qol-cli/src/commands/emu/qmp.rs` (verb methods + tests)
- Modify: `tools/qol-cli/src/commands/emu.rs` (`mod control;`, dispatch, `pub(crate) fn unix_millis`)

- [ ] **Step 1: Write the failing tests**

In `qmp.rs` tests (fake-server pattern from Task 1; the helper closure below is shared by copy-paste, fine at this size):

```rust
    fn fake_server(
        replies: Vec<&'static str>,
        assert_lines: fn(usize, &str),
    ) -> (std::thread::JoinHandle<()>, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
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
            writeln!(stream, r#"{{"return":{{}}}}"#).unwrap();
            for (index, reply) in replies.into_iter().enumerate() {
                line.clear();
                reader.read_line(&mut line).unwrap();
                assert_lines(index, &line);
                writeln!(stream, "{reply}").unwrap();
            }
        });
        (handle, port)
    }

    #[test]
    fn send_keys_builds_qcode_chord() {
        let (server, port) = fake_server(vec![r#"{"return":{}}"#], |_, line| {
            assert!(line.contains(r#""execute":"send-key""#), "line: {line}");
            assert!(
                line.contains(r#"{"data":"ctrl","type":"qcode"}"#),
                "line: {line}"
            );
            assert!(
                line.contains(r#"{"data":"c","type":"qcode"}"#),
                "line: {line}"
            );
        });
        let mut client = connect(port, Duration::from_secs(2)).unwrap();
        client
            .send_keys(&["ctrl".to_string(), "c".to_string()])
            .unwrap();
        server.join().unwrap();
    }

    #[test]
    fn screendump_sends_filename() {
        let (server, port) = fake_server(vec![r#"{"return":{}}"#], |_, line| {
            assert!(line.contains(r#""execute":"screendump""#), "line: {line}");
            assert!(line.contains("shot.ppm"), "line: {line}");
        });
        let mut client = connect(port, Duration::from_secs(2)).unwrap();
        client.screendump(Path::new("/a/b/shot.ppm")).unwrap();
        server.join().unwrap();
    }
```

Refactor the Task 1 tests (`execute_sends_arguments_payload`, `wait_event_skips_unrelated_lines_until_match`) onto `fake_server` if it falls out naturally; do not force it. Add `use std::path::Path;` to the test imports.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p qol qmp`
Expected: compile errors, `send_keys`/`screendump` missing.

- [ ] **Step 3: Implement the qmp verb methods**

```rust
    pub(crate) fn screendump(&mut self, path: &Path) -> Result<()> {
        self.execute(
            "screendump",
            Some(serde_json::json!({"filename": path.display().to_string()})),
        )?;
        Ok(())
    }

    pub(crate) fn send_keys(&mut self, keys: &[String]) -> Result<()> {
        let chord: Vec<Value> = keys
            .iter()
            .map(|key| serde_json::json!({"type": "qcode", "data": key}))
            .collect();
        self.execute("send-key", Some(serde_json::json!({"keys": chord})))?;
        Ok(())
    }
```

Add `use std::path::Path;` to `qmp.rs` imports.

- [ ] **Step 4: Implement control.rs and dispatch**

`control.rs`:

```rust
use anyhow::{anyhow, bail, Result};
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use crate::progress::{print_hint, print_title, step_label, StepKind};
use crate::workspace::repo_root;

use super::{live, qmp, unix_millis};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) fn cmd_shot(args: &[OsString], verbose: bool) -> Result<()> {
    let id = single_id(args, "shot")?;
    print_title("qol emu shot");
    print_hint(verbose);
    let live = live::find(&runs_root()?, &id)?;
    let mut client = qmp::connect(live.qmp_port, CONTROL_TIMEOUT)?;
    let path = live.run_dir.join(format!("screenshot-{}.ppm", unix_millis()?));
    client.screendump(&path)?;
    step_label("shot", StepKind::Success, &path.display().to_string());
    Ok(())
}

pub(crate) fn cmd_key(args: &[OsString], verbose: bool) -> Result<()> {
    let (id, keys) = id_and_rest(args, "key", "<qcode>...")?;
    print_title("qol emu key");
    print_hint(verbose);
    let live = live::find(&runs_root()?, &id)?;
    let mut client = qmp::connect(live.qmp_port, CONTROL_TIMEOUT)?;
    client.send_keys(&keys)?;
    step_label("key", StepKind::Success, &keys.join("+"));
    Ok(())
}

pub(crate) fn cmd_down(args: &[OsString], verbose: bool) -> Result<()> {
    let id = single_id(args, "down")?;
    print_title("qol emu down");
    print_hint(verbose);
    let live = live::find(&runs_root()?, &id)?;
    let mut client = qmp::connect(live.qmp_port, CONTROL_TIMEOUT)?;
    client.fire("quit")?;
    step_label("down", StepKind::Success, "quit sent; up will finalize the report");
    Ok(())
}

fn runs_root() -> Result<PathBuf> {
    Ok(repo_root()?.join("target/qol-emu"))
}

fn single_id(args: &[OsString], command: &str) -> Result<String> {
    let [id] = args else {
        bail!("usage: qol emu {command} <environment>");
    };
    utf8(id)
}

fn id_and_rest(args: &[OsString], command: &str, rest_usage: &str) -> Result<(String, Vec<String>)> {
    let Some((id, rest)) = args.split_first() else {
        bail!("usage: qol emu {command} <environment> {rest_usage}");
    };
    if rest.is_empty() {
        bail!("usage: qol emu {command} <environment> {rest_usage}");
    }
    let rest = rest.iter().map(utf8).collect::<Result<Vec<_>>>()?;
    Ok((utf8(id)?, rest))
}

fn utf8(value: &OsString) -> Result<String> {
    value
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("argument is not valid UTF-8"))
}
```

In `emu.rs`: add `mod control;`, make `unix_millis` `pub(crate)` (it is `fn unix_millis()` today), and extend the dispatch:

```rust
        "list" => cmd_list(rest, verbose),
        "doctor" => cmd_doctor(rest, verbose),
        "up" => cmd_up(rest, verbose),
        "shot" => control::cmd_shot(rest, verbose),
        "key" => control::cmd_key(rest, verbose),
        "down" => control::cmd_down(rest, verbose),
```

`down` uses `fire` (write-only) because QEMU may close the socket before the `quit` reply arrives; the blocking `up` process owns the exit path and writes the final report, so `down` must not race it for a reply.

- [ ] **Step 5: Run the gate.** Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add tools/qol-cli/src/commands/emu/control.rs tools/qol-cli/src/commands/emu/qmp.rs tools/qol-cli/src/commands/emu.rs
git commit -m "feat(emu): shot, key and down verbs against the live qmp socket"
```

---

### Task 5: insert and pull (USB stick hot-plug)

**Files:**
- Modify: `tools/qol-cli/src/commands/emu/qmp.rs`
- Modify: `tools/qol-cli/src/commands/emu/machine.rs`
- Modify: `tools/qol-cli/src/commands/emu/control.rs`
- Modify: `tools/qol-cli/src/commands/emu.rs` (dispatch)

- [ ] **Step 1: Write the failing tests**

`qmp.rs`:

```rust
    #[test]
    fn attach_usb_stick_adds_blockdev_then_device() {
        let (server, port) = fake_server(
            vec![r#"{"return":{}}"#, r#"{"return":{}}"#],
            |index, line| match index {
                0 => {
                    assert!(line.contains(r#""execute":"blockdev-add""#), "line: {line}");
                    assert!(line.contains(r#""node-name":"qolusb""#), "line: {line}");
                    assert!(line.contains("usb-stick.raw"), "line: {line}");
                }
                1 => {
                    assert!(line.contains(r#""execute":"device_add""#), "line: {line}");
                    assert!(line.contains(r#""driver":"usb-storage""#), "line: {line}");
                    assert!(line.contains(r#""bus":"xhci.0""#), "line: {line}");
                }
                other => panic!("unexpected command index {other}: {line}"),
            },
        );
        let mut client = connect(port, Duration::from_secs(2)).unwrap();
        client
            .attach_usb_stick(Path::new("/a/b/usb-stick.raw"))
            .unwrap();
        server.join().unwrap();
    }

    #[test]
    fn detach_usb_stick_deletes_device_waits_then_drops_blockdev() {
        let (server, port) = fake_server(
            vec![
                r#"{"return":{}}
{"event":"DEVICE_DELETED","data":{"device":"qolusbdev"},"timestamp":{"seconds":0,"microseconds":0}}"#,
                r#"{"return":{}}"#,
            ],
            |index, line| match index {
                0 => assert!(line.contains(r#""execute":"device_del""#), "line: {line}"),
                1 => assert!(line.contains(r#""execute":"blockdev-del""#), "line: {line}"),
                other => panic!("unexpected command index {other}: {line}"),
            },
        );
        let mut client = connect(port, Duration::from_secs(2)).unwrap();
        client.detach_usb_stick().unwrap();
        server.join().unwrap();
    }
```

(The first reply string is two lines: the `device_del` return, then the event `wait_event` consumes. `writeln!` writes both.)

`machine.rs`:

```rust
    #[test]
    fn ensure_usb_stick_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("qol-emu-stick-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("usb-stick.raw"), b"existing").unwrap();
        let stick = ensure_usb_stick(&dir, Path::new("/nonexistent/qemu-img")).unwrap();
        assert_eq!(stick, dir.join("usb-stick.raw"));
        assert_eq!(fs::read(&stick).unwrap(), b"existing");
        fs::remove_dir_all(&dir).unwrap();
    }
```

(Existing file short-circuits before `qemu-img` runs, so the bogus path proves the idempotence branch.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p qol -- attach_usb detach_usb ensure_usb`
Expected: compile errors.

- [ ] **Step 3: Implement**

`qmp.rs`:

```rust
    pub(crate) fn attach_usb_stick(&mut self, image: &Path) -> Result<()> {
        self.execute(
            "blockdev-add",
            Some(serde_json::json!({
                "driver": "raw",
                "node-name": "qolusb",
                "file": {"driver": "file", "filename": image.display().to_string()},
            })),
        )?;
        self.execute(
            "device_add",
            Some(serde_json::json!({
                "driver": "usb-storage",
                "id": "qolusbdev",
                "bus": "xhci.0",
                "drive": "qolusb",
            })),
        )?;
        Ok(())
    }

    pub(crate) fn detach_usb_stick(&mut self) -> Result<()> {
        self.execute("device_del", Some(serde_json::json!({"id": "qolusbdev"})))?;
        self.wait_event("DEVICE_DELETED", Duration::from_secs(5))?;
        self.execute(
            "blockdev-del",
            Some(serde_json::json!({"node-name": "qolusb"})),
        )?;
        Ok(())
    }
```

`machine.rs`:

```rust
pub(crate) fn ensure_usb_stick(run_dir: &Path, qemu_img: &Path) -> Result<PathBuf> {
    let stick = run_dir.join("usb-stick.raw");
    if stick.is_file() {
        return Ok(stick);
    }
    let status = Command::new(qemu_img)
        .arg("create")
        .arg("-f")
        .arg("raw")
        .arg(&stick)
        .arg("16M")
        .status()
        .with_context(|| format!("failed to run {}", qemu_img.display()))?;
    if !status.success() {
        bail!("qemu-img create failed for {}", stick.display());
    }
    Ok(stick)
}
```

(Add `bail` to the `anyhow` imports in `machine.rs`.)

`control.rs` handlers + dispatch arms `"insert"`/`"pull"`:

```rust
pub(crate) fn cmd_insert(args: &[OsString], verbose: bool) -> Result<()> {
    let id = single_id(args, "insert")?;
    print_title("qol emu insert");
    print_hint(verbose);
    let live = live::find(&runs_root()?, &id)?;
    let qemu_img = super::find_on_path("qemu-img")
        .ok_or_else(|| anyhow!("missing qemu-img on PATH"))?;
    let stick = machine::ensure_usb_stick(&live.run_dir, &qemu_img)?;
    let mut client = qmp::connect(live.qmp_port, CONTROL_TIMEOUT)?;
    client.attach_usb_stick(&stick)?;
    step_label("insert", StepKind::Success, &stick.display().to_string());
    Ok(())
}

pub(crate) fn cmd_pull(args: &[OsString], verbose: bool) -> Result<()> {
    let id = single_id(args, "pull")?;
    print_title("qol emu pull");
    print_hint(verbose);
    let live = live::find(&runs_root()?, &id)?;
    let mut client = qmp::connect(live.qmp_port, CONTROL_TIMEOUT)?;
    client.detach_usb_stick()?;
    step_label("pull", StepKind::Success, "usb stick detached");
    Ok(())
}
```

Add `machine` to the `super::{...}` import in `control.rs` and make `find_on_path` `pub(crate)` in `emu.rs` (it is private today). Double-insert without a pull surfaces QEMU's own duplicate `node-name` error verbatim - honest, no masking.

- [ ] **Step 4: Run the gate.** Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add tools/qol-cli/src/commands/emu/qmp.rs tools/qol-cli/src/commands/emu/machine.rs tools/qol-cli/src/commands/emu/control.rs tools/qol-cli/src/commands/emu.rs
git commit -m "feat(emu): usb stick insert and pull over qmp hot-plug"
```

---

### Task 6: snap (disk snapshot) and image-aware teardown

**Files:**
- Modify: `tools/qol-cli/src/commands/emu/qmp.rs`
- Modify: `tools/qol-cli/src/commands/emu/machine.rs`
- Modify: `tools/qol-cli/src/commands/emu/discovery/mod.rs` (re-export)
- Modify: `tools/qol-cli/src/commands/emu/control.rs` + `emu.rs` dispatch

- [ ] **Step 1: Write the failing tests**

`qmp.rs`:

```rust
    #[test]
    fn disk_snapshot_targets_qoldisk() {
        let (server, port) = fake_server(vec![r#"{"return":{}}"#], |_, line| {
            assert!(
                line.contains(r#""execute":"blockdev-snapshot-sync""#),
                "line: {line}"
            );
            assert!(line.contains(r#""device":"qoldisk""#), "line: {line}");
            assert!(line.contains("overlay-snap"), "line: {line}");
        });
        let mut client = connect(port, Duration::from_secs(2)).unwrap();
        client
            .disk_snapshot(Path::new("/a/b/overlay-snap-1.qcow2"))
            .unwrap();
        server.join().unwrap();
    }
```

`machine.rs` - replace `teardown_removes_overlay_and_keeps_artifacts`:

```rust
    #[test]
    fn teardown_removes_disk_images_and_keeps_evidence() {
        let dir = std::env::temp_dir().join(format!("qol-emu-teardown-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let files = [
            "overlay.qcow2",
            "overlay-snap-1.qcow2",
            "usb-stick.raw",
            "report.json",
            "qemu-command.txt",
            "screenshot-1.ppm",
        ];
        for name in files {
            fs::write(dir.join(name), b"x").unwrap();
        }
        let removed = teardown(&dir).unwrap();
        let mut expected_removed = vec![
            dir.join("overlay-snap-1.qcow2"),
            dir.join("overlay.qcow2"),
            dir.join("usb-stick.raw"),
        ];
        expected_removed.sort();
        assert_eq!(removed, expected_removed);
        let expectations = [
            ("overlay.qcow2", false),
            ("overlay-snap-1.qcow2", false),
            ("usb-stick.raw", false),
            ("report.json", true),
            ("qemu-command.txt", true),
            ("screenshot-1.ppm", true),
        ];
        for (name, should_exist) in expectations {
            assert_eq!(dir.join(name).exists(), should_exist, "file: {name}");
        }
        fs::remove_dir_all(&dir).unwrap();
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p qol -- disk_snapshot teardown`
Expected: compile error (`disk_snapshot` missing) and teardown assertion failure (only `overlay.qcow2` removed).

- [ ] **Step 3: Implement**

`qmp.rs`:

```rust
    pub(crate) fn disk_snapshot(&mut self, snapshot_file: &Path) -> Result<()> {
        self.execute(
            "blockdev-snapshot-sync",
            Some(serde_json::json!({
                "device": "qoldisk",
                "snapshot-file": snapshot_file.display().to_string(),
                "format": "qcow2",
            })),
        )?;
        Ok(())
    }
```

`discovery/mod.rs`: add `pub(crate) use filesystem::is_vm_image_path;`.

`machine.rs`: delete `DISPOSABLE_FILES`, rewrite teardown:

```rust
use super::discovery::is_vm_image_path;

pub(crate) fn teardown(run_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    let entries = fs::read_dir(run_dir)
        .with_context(|| format!("failed to read {}", run_dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && is_vm_image_path(&path) {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
            removed.push(path);
        }
    }
    removed.sort();
    Ok(removed)
}
```

`control.rs` + dispatch arm `"snap"`:

```rust
pub(crate) fn cmd_snap(args: &[OsString], verbose: bool) -> Result<()> {
    let id = single_id(args, "snap")?;
    print_title("qol emu snap");
    print_hint(verbose);
    let live = live::find(&runs_root()?, &id)?;
    let mut client = qmp::connect(live.qmp_port, CONTROL_TIMEOUT)?;
    let snapshot = live
        .run_dir
        .join(format!("overlay-snap-{}.qcow2", unix_millis()?));
    client.disk_snapshot(&snapshot)?;
    step_label("snap", StepKind::Success, &snapshot.display().to_string());
    step_label(
        "frozen",
        StepKind::Info,
        "previous overlay is now read-only and safe for host inspection",
    );
    Ok(())
}
```

Semantics worth knowing: `blockdev-snapshot-sync` makes the *new* file the active write layer; the previously active overlay freezes as its backing file. That frozen file is the `DiskSnapshot` the spec's `list_qol_traces` will read in M3. Teardown disposes of the whole chain because all of it lives in the run dir.

- [ ] **Step 4: Run the gate.** Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add tools/qol-cli/src/commands/emu/qmp.rs tools/qol-cli/src/commands/emu/machine.rs tools/qol-cli/src/commands/emu/discovery/mod.rs tools/qol-cli/src/commands/emu/control.rs tools/qol-cli/src/commands/emu.rs
git commit -m "feat(emu): disk snapshot verb and image-aware teardown"
```

---

### Task 7: Help text, runtime verification, docs

**Files:**
- Modify: `tools/qol-cli/src/commands/emu.rs` (`emu_help_text`)
- Modify: `apps/qol-tray/skills/qol-cli-commands/SKILL.md`
- Modify: `docs/superpowers/specs/2026-06-10-emu-test-harness-design.md` (status note)

- [ ] **Step 1: Update help text**

```rust
fn emu_help_text() -> &'static str {
    "qol emu commands:\n  qol emu list\n  qol emu doctor\n  qol emu up <environment>\n  qol emu shot <environment>\n  qol emu key <environment> <qcode>...\n  qol emu insert <environment>\n  qol emu pull <environment>\n  qol emu snap <environment>\n  qol emu down <environment>\n\nControl verbs target the newest running `qol emu up` for that environment.\n\nEmus are discovered from libvirt/QEMU domains plus optional local config:\n  ~/.config/qol-tray/emu.toml\n\nExample config:\n  [images]\n  my-windows = \"/path/to/windows.qcow2\"\n"
}
```

- [ ] **Step 2: Runtime verification against a real QEMU**

```bash
qemu-img create -f qcow2 ~/VMs/scratch.qcow2 1G
cargo run -p qol -- emu up scratch &
sleep 8
cargo run -p qol -- emu shot scratch
cargo run -p qol -- emu key scratch ret
cargo run -p qol -- emu insert scratch
cargo run -p qol -- emu pull scratch
cargo run -p qol -- emu snap scratch
cargo run -p qol -- emu shot scratch
cargo run -p qol -- emu down scratch
wait
```

Expected: every verb prints a green step; `up` exits cleanly after `down`. Then inspect the newest `target/qol-emu/scratch-*/`:

- `report.json` has `"status": "pass"` and `teardown.removed` lists `overlay.qcow2`, `overlay-snap-*.qcow2`, `usb-stick.raw`.
- Two `screenshot-*.ppm` files remain (open one; SeaBIOS text should be visible).
- No `*.qcow2` / `*.raw` files remain.

Known risk to verify live: if `DEVICE_DELETED` does not arrive on `pull` (no guest OS to cooperate), the verb fails with the timeout message. USB unplug is host-side, so it should fire; if it does not, stop and reassess rather than masking.

**Cleanup (mandatory, this polluted the dashboard last time):**

```bash
rm ~/VMs/scratch.qcow2
cargo run -p qol -- emu list
```

Expected: `no emus found` (or only the user's real images).

- [ ] **Step 3: Update SKILL.md**

In the `## Emu` section of `apps/qol-tray/skills/qol-cli-commands/SKILL.md`: change "M1 (launch) is implemented; M2 ... is next" to say M2 control verbs are implemented and list them:

```text
- `qol emu shot <id>`: QMP screendump into the run dir (kept as evidence).
- `qol emu key <id> <qcode>...`: send one key chord (e.g. `ctrl alt delete`).
- `qol emu insert <id>` / `qol emu pull <id>`: hot-plug a scratch USB stick (xhci + usb-storage).
- `qol emu snap <id>`: freeze the active overlay via blockdev-snapshot-sync; the frozen file is host-readable.
- `qol emu down <id>`: send `quit`; the blocking `up` finalizes the report and teardown.
```

Also note: teardown now removes every disk image in the run dir (`overlay*.qcow2`, `usb-stick.raw`); screenshots and reports stay. Remaining M2 item: arch-aware hvf/whpx finish (own plan).

- [ ] **Step 4: Update the spec status**

In the spec's `## Status` section add one line: "M2 control surface implemented (shot/key/insert/pull/snap/down); hvf/whpx finish pending."

- [ ] **Step 5: Run the full gate one last time.** Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add tools/qol-cli/src/commands/emu.rs apps/qol-tray/skills/qol-cli-commands/SKILL.md docs/superpowers/specs/2026-06-10-emu-test-harness-design.md
git commit -m "docs(emu): document m2 control verbs and verification findings"
```
