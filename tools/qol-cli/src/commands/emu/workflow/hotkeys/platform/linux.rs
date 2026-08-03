use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use qol_conventions::{local_base_url, DEFAULT_PORT, TRACE_LOG_PATH};
use qol_dev_guest::{GuestControlClient, ProcessOutcome};

use crate::commands::emu::{qmp, BootedVm};
use crate::progress::{step_label, StepKind};

use super::desktop::{
    command, connect_desktop_guest, current_trace_cursor, install_payload,
    launch_tray_and_wait_api, require_exec, spawn, start_tray_and_wait_plugin, wait_for_command,
    wait_for_probe_fields, wait_for_window_id,
};
use super::Verdict;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const ACTION_TIMEOUT: Duration = Duration::from_secs(20);
const KEY_SETTLE: Duration = Duration::from_millis(140);
const DISABLED_SETTLE: Duration = Duration::from_millis(500);
const CYCLE_COUNT: usize = 40;
const DISABLED_CYCLE_COUNT: usize = 5;
const RELOAD_FLOOD_COUNT: usize = 40;
const LAUNCHER_UID: &str = "5cc75f62-2e3b-463c-ac7b-ae269cff1ef1";
const PASSTHROUGH_INPUT_PATH: &str = "/tmp/qol-hotkey-passthrough.txt";
const PASSTHROUGH_WINDOW_TITLE: &str = "qol-hotkey-passthrough";
const PORT_CLOSED_SCRIPT: &str = r#"
import socket
import sys

handle = socket.socket()
handle.settimeout(0.5)
result = handle.connect_ex(("127.0.0.1", int(sys.argv[1])))
handle.close()
raise SystemExit(0 if result != 0 else 1)
"#;
const PROBE_COUNT_SCRIPT: &str = r#"
import pathlib
import sys

lines = pathlib.Path(sys.argv[1]).read_text(errors="replace").splitlines()
tag = " " + sys.argv[2] + " "
required = sys.argv[3:]
print(sum(tag in line and all(field in line for field in required) for line in lines))
"#;

struct HttpResponse {
    status: u16,
    body: String,
}

struct StormEvidence {
    attempts: usize,
    dispatches: usize,
    fds_before: usize,
    fds_after: usize,
}

pub(super) fn run(vm: &BootedVm) -> Result<Verdict> {
    let mut guest = connect_desktop_guest(vm)?;
    install_payload(&mut guest)?;
    let mut auth = start_tray_and_wait_plugin(&mut guest, "plugin-launcher")?;
    let backend = hotkey_backend(&mut guest)?;
    let baseline = require_status(
        request(&mut guest, &auth, "GET", "/api/hotkeys", None)?,
        200,
    )?;
    let mut qmp = qmp::connect_verified(vm.qmp_port, Duration::from_secs(10), &vm.run_id)?;
    require_passthrough_keys(&mut guest, &mut qmp)?;

    let storm = run_storm(&mut guest, &mut qmp, &mut auth, &backend);
    let restore = set_hotkeys(&mut guest, &auth, &baseline);
    let final_config = request(&mut guest, &auth, "GET", "/api/hotkeys", None)
        .and_then(|response| require_status(response, 200));
    let evidence = match (storm, restore, final_config) {
        (Ok(evidence), Ok(()), Ok(final_config)) => {
            require_json_equal(&baseline, &final_config, "hotkey baseline restoration")?;
            evidence
        }
        (Err(error), Err(cleanup), _) => {
            bail!("{error:#}; hotkey baseline restoration also failed: {cleanup:#}")
        }
        (Err(error), _, _) => return Err(error),
        (Ok(_), Err(cleanup), _) => return Err(cleanup),
        (Ok(_), Ok(()), Err(error)) => return Err(error),
    };

    let artifacts_dir = vm.run_dir.join("artifacts");
    std::fs::create_dir_all(&artifacts_dir)
        .with_context(|| format!("failed to create {}", artifacts_dir.display()))?;
    let final_path = artifacts_dir.join("hotkeys-final.json");
    std::fs::write(&final_path, &baseline)
        .with_context(|| format!("failed to write {}", final_path.display()))?;
    let trace = require_exec(
        &mut guest,
        command("/usr/bin/grep", &["HOTKEY_", TRACE_LOG_PATH]),
        COMMAND_TIMEOUT,
    )?;
    let trace_path = artifacts_dir.join("hotkeys-trace.log");
    std::fs::write(&trace_path, &trace.stdout)
        .with_context(|| format!("failed to write {}", trace_path.display()))?;
    let evidence_path = artifacts_dir.join("hotkeys-environment.json");
    let evidence_json = serde_json::json!({
        "backend": backend,
        "attempts": evidence.attempts,
        "dispatches": evidence.dispatches,
        "fds_before": evidence.fds_before,
        "fds_after": evidence.fds_after,
        "native_backend_exercised": backend == "native",
        "passthrough_keys": ["caps-lock", "control", "iso-102nd"],
    });
    std::fs::write(&evidence_path, serde_json::to_vec_pretty(&evidence_json)?)
        .with_context(|| format!("failed to write {}", evidence_path.display()))?;

    step_label(
        "storm",
        StepKind::Success,
        &format!(
            "layout passthrough, {CYCLE_COUNT} physical cycles, disable/migrate/duplicate guards, {RELOAD_FLOOD_COUNT} reloads, restart persistence, and cleanup passed ({backend})"
        ),
    );
    Ok(Verdict {
        pass: true,
        traces: trace.stdout.lines().map(str::to_string).collect(),
        artifacts: vec![final_path, trace_path, evidence_path],
    })
}

pub(super) fn require_passthrough_keys(
    guest: &mut GuestControlClient,
    qmp: &mut qmp::QmpClient,
) -> Result<()> {
    require_caps_lock_passthrough(guest, qmp)?;
    require_exec(
        guest,
        command("/usr/bin/rm", &["-f", PASSTHROUGH_INPUT_PATH]),
        COMMAND_TIMEOUT,
    )?;
    let script = format!("IFS= read -e -r line; printf '%s' \"$line\" > {PASSTHROUGH_INPUT_PATH}");
    spawn(
        guest,
        command(
            "/usr/bin/xterm",
            &[
                "-T",
                PASSTHROUGH_WINDOW_TITLE,
                "-e",
                "/bin/bash",
                "-lc",
                &script,
            ],
        ),
    )?;
    let window_id = wait_for_window_id(guest, PASSTHROUGH_WINDOW_TITLE, ACTION_TIMEOUT)?;
    require_exec(
        guest,
        command(
            "/usr/bin/xdotool",
            &["windowactivate", "--sync", &window_id],
        ),
        COMMAND_TIMEOUT,
    )?;
    key(qmp, &["x"])?;
    key(qmp, &["ctrl", "a"])?;
    key(qmp, &["shift", "less"])?;
    key(qmp, &["ret"])?;
    let outcome = wait_for_command(
        guest,
        command("/usr/bin/cat", &[PASSTHROUGH_INPUT_PATH]),
        ACTION_TIMEOUT,
        |outcome| outcome.stdout == ">x",
        "Control and the ISO 102nd key to pass through evdev",
    )?;
    if outcome.stdout != ">x" {
        bail!(
            "passthrough input mismatch: expected >x, got {:?}",
            outcome.stdout
        );
    }
    step_label(
        "input",
        StepKind::Success,
        "Caps Lock, Control, and the ISO 102nd key reached X11",
    );
    Ok(())
}

fn require_caps_lock_passthrough(
    guest: &mut GuestControlClient,
    qmp: &mut qmp::QmpClient,
) -> Result<()> {
    if caps_lock_enabled(guest)? {
        key(qmp, &["caps_lock"])?;
        wait_for_caps_lock(guest, false)?;
    }
    key(qmp, &["caps_lock"])?;
    wait_for_caps_lock(guest, true)?;
    key(qmp, &["caps_lock"])?;
    wait_for_caps_lock(guest, false)
}

fn caps_lock_enabled(guest: &mut GuestControlClient) -> Result<bool> {
    let outcome = require_exec(guest, command("/usr/bin/xset", &["q"]), COMMAND_TIMEOUT)?;
    parse_caps_lock(&outcome.stdout).context("xset output did not report Caps Lock state")
}

fn wait_for_caps_lock(guest: &mut GuestControlClient, expected: bool) -> Result<()> {
    wait_for_command(
        guest,
        command("/usr/bin/xset", &["q"]),
        ACTION_TIMEOUT,
        |outcome| parse_caps_lock(&outcome.stdout) == Some(expected),
        &format!("Caps Lock to turn {}", if expected { "on" } else { "off" }),
    )?;
    Ok(())
}

fn parse_caps_lock(output: &str) -> Option<bool> {
    output
        .split_ascii_whitespace()
        .collect::<Vec<_>>()
        .windows(3)
        .find_map(|fields| match fields {
            ["Caps", "Lock:", "on"] => Some(true),
            ["Caps", "Lock:", "off"] => Some(false),
            _ => None,
        })
}

fn run_storm(
    guest: &mut GuestControlClient,
    qmp: &mut qmp::QmpClient,
    auth: &mut String,
    backend: &str,
) -> Result<StormEvidence> {
    let fds_before = tray_fd_count(guest)?;
    let dispatches_before = dispatch_count(guest, "action=open")?;
    register_and_require(
        guest,
        auth,
        &hotkey_config("hotkey-storm-f12", "F12", "open", true),
        "F12",
        "open",
        backend,
    )?;
    require_no_registration_errors(guest, auth)?;
    require_launcher_window_after_key(guest, qmp)?;

    let before = dispatch_count(guest, "action=open")?;
    for _ in 0..CYCLE_COUNT {
        key(qmp, &["f12"])?;
        key(qmp, &["esc"])?;
    }
    let after = wait_for_dispatch_count(guest, "action=open", before + CYCLE_COUNT)?;
    if after != before + CYCLE_COUNT {
        bail!(
            "hotkey cycle count drifted: expected {}, got {after}",
            before + CYCLE_COUNT
        );
    }

    let cursor = current_trace_cursor(guest)?;
    set_hotkeys(
        guest,
        auth,
        &hotkey_config("hotkey-storm-f12", "F12", "open", false),
    )?;
    wait_for_binding_reload(
        guest,
        cursor,
        backend,
        Some(("unregistered", "F12", "open")),
    )?;
    let disabled_before = dispatch_count(guest, "action=open")?;
    for _ in 0..DISABLED_CYCLE_COUNT {
        key(qmp, &["f12"])?;
    }
    thread::sleep(DISABLED_SETTLE);
    let disabled_after = dispatch_count(guest, "action=open")?;
    if disabled_after != disabled_before {
        bail!(
            "disabled F12 dispatched {} time(s)",
            disabled_after - disabled_before
        );
    }

    register_and_require(
        guest,
        auth,
        &hotkey_config("hotkey-storm-modified", "Ctrl+Shift+F12", "open", true),
        "Ctrl_Shift_F12",
        "open",
        backend,
    )?;
    let migrated_before = dispatch_count(guest, "action=open")?;
    key(qmp, &["f12"])?;
    thread::sleep(DISABLED_SETTLE);
    if dispatch_count(guest, "action=open")? != migrated_before {
        bail!("old F12 chord remained active after migration");
    }
    key(qmp, &["ctrl", "shift", "f12"])?;
    let migrated_after = wait_for_dispatch_count(guest, "action=open", migrated_before + 1)?;
    if migrated_after != migrated_before + 1 {
        bail!("migrated Ctrl+Shift+F12 chord dispatched more than once");
    }

    test_rejections_leave_state_untouched(guest, auth)?;
    flood_reloads(guest, auth, backend)?;
    require_no_registration_errors(guest, auth)?;
    let flood_before = dispatch_count(guest, "action=open")?;
    key(qmp, &["f12"])?;
    let flood_after = wait_for_dispatch_count(guest, "action=open", flood_before + 1)?;
    if flood_after != flood_before + 1 {
        bail!("final binding after reload flood dispatched more than once");
    }

    let fds_after = tray_fd_count(guest)?;
    if fds_after > fds_before.saturating_add(8) {
        bail!("qol-tray file descriptors grew from {fds_before} to {fds_after}");
    }

    restart_and_require_persistence(guest, qmp, auth, backend)?;
    let dispatches_after = dispatch_count(guest, "action=open")?;
    let dispatches = dispatches_after.saturating_sub(dispatches_before);
    if dispatches != CYCLE_COUNT + 4 {
        bail!(
            "hotkey storm dispatched {dispatches} times; expected {}",
            CYCLE_COUNT + 4
        );
    }
    Ok(StormEvidence {
        attempts: CYCLE_COUNT + DISABLED_CYCLE_COUNT + 5,
        dispatches,
        fds_before,
        fds_after,
    })
}

fn require_launcher_window_after_key(
    guest: &mut GuestControlClient,
    qmp: &mut qmp::QmpClient,
) -> Result<()> {
    key(qmp, &["f12"])?;
    wait_for_command(
        guest,
        command("/usr/bin/xdotool", &["search", "--name", "^qol-launcher@"]),
        ACTION_TIMEOUT,
        |outcome| !outcome.stdout.trim().is_empty(),
        "the launcher window opened by the hotkey",
    )?;
    key(qmp, &["esc"])
}

fn test_rejections_leave_state_untouched(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let before = require_status(request(guest, auth, "GET", "/api/hotkeys", None)?, 200)?;
    require_status(request(guest, auth, "PUT", "/api/hotkeys", Some("{"))?, 400)?;
    let duplicate = serde_json::json!({
        "hotkeys": [
            binding("duplicate-first", "F12", "open", true),
            binding("duplicate-second", "f12", "settings", true),
        ]
    })
    .to_string();
    let response = request(guest, auth, "PUT", "/api/hotkeys", Some(&duplicate))?;
    if response.status != 400 || !response.body.contains("Duplicate enabled hotkey chord") {
        bail!(
            "duplicate chord was not rejected precisely: HTTP {} {}",
            response.status,
            response.body
        );
    }
    let after = require_status(request(guest, auth, "GET", "/api/hotkeys", None)?, 200)?;
    require_json_equal(&before, &after, "rejected hotkey writes")?;
    Ok(())
}

fn flood_reloads(guest: &mut GuestControlClient, auth: &str, backend: &str) -> Result<()> {
    for index in 0..RELOAD_FLOOD_COUNT {
        set_hotkeys(
            guest,
            auth,
            &hotkey_config("hotkey-storm-flood", "F12", "open", index % 2 == 0),
        )?;
    }
    register_and_require(
        guest,
        auth,
        &hotkey_config("hotkey-storm-final", "F12", "open", true),
        "F12",
        "open",
        backend,
    )
}

fn restart_and_require_persistence(
    guest: &mut GuestControlClient,
    qmp: &mut qmp::QmpClient,
    auth: &mut String,
    backend: &str,
) -> Result<()> {
    let before_pid = tray_pid(guest)?;
    let cursor = current_trace_cursor(guest)?;
    require_status(request(guest, auth, "POST", "/api/shutdown", None)?, 202)?;
    let port = DEFAULT_PORT.to_string();
    wait_for_command(
        guest,
        command("/usr/bin/python3", &["-c", PORT_CLOSED_SCRIPT, &port]),
        ACTION_TIMEOUT,
        |_| true,
        "the tray HTTP port to close",
    )?;
    *auth = launch_tray_and_wait_api(guest)?;
    let after_pid = tray_pid(guest)?;
    if before_pid == after_pid {
        bail!("qol-tray did not restart during hotkey persistence test");
    }
    if backend == "native" {
        wait_for_probe_fields(
            guest,
            cursor,
            "HOTKEY_BINDINGS",
            &["backend=native", "phase=startup", "count=1"],
            ACTION_TIMEOUT,
        )?;
    } else {
        wait_for_probe_fields(
            guest,
            cursor,
            "HOTKEY_REGISTRATION",
            &["result=registered", "key=F12", "action=open"],
            ACTION_TIMEOUT,
        )?;
    }
    let before = dispatch_count(guest, "action=open")?;
    key(qmp, &["f12"])?;
    let after = wait_for_dispatch_count(guest, "action=open", before + 1)?;
    if after != before + 1 {
        bail!("persisted hotkey dispatched more than once after restart");
    }
    Ok(())
}

fn register_and_require(
    guest: &mut GuestControlClient,
    auth: &str,
    config: &str,
    key: &str,
    action: &str,
    backend: &str,
) -> Result<()> {
    let cursor = current_trace_cursor(guest)?;
    set_hotkeys(guest, auth, config)?;
    wait_for_binding_reload(guest, cursor, backend, Some(("registered", key, action)))?;
    Ok(())
}

fn wait_for_binding_reload(
    guest: &mut GuestControlClient,
    cursor: super::desktop::TraceCursor,
    backend: &str,
    fallback: Option<(&str, &str, &str)>,
) -> Result<()> {
    if backend == "native" {
        wait_for_probe_fields(
            guest,
            cursor,
            "HOTKEY_BINDINGS",
            &["backend=native", "phase=reload", "result=loaded"],
            ACTION_TIMEOUT,
        )?;
        return Ok(());
    }
    let (result, key, action) = fallback.context("fallback binding evidence was omitted")?;
    let result = format!("result={result}");
    let key = format!("key={key}");
    let action = format!("action={action}");
    let mut fields = vec![result.as_str(), key.as_str()];
    if result != "result=unregistered" {
        fields.push(action.as_str());
    }
    wait_for_probe_fields(
        guest,
        cursor,
        "HOTKEY_REGISTRATION",
        &fields,
        ACTION_TIMEOUT,
    )?;
    Ok(())
}

fn set_hotkeys(guest: &mut GuestControlClient, auth: &str, body: &str) -> Result<()> {
    require_status(
        request(guest, auth, "PUT", "/api/hotkeys", Some(body))?,
        200,
    )?;
    Ok(())
}

fn require_no_registration_errors(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let body = require_status(
        request(guest, auth, "GET", "/api/hotkeys/errors", None)?,
        200,
    )?;
    let errors: Vec<serde_json::Value> =
        serde_json::from_str(&body).context("hotkey error response was not an array")?;
    if !errors.is_empty() {
        bail!("hotkey registration errors were reported: {body}");
    }
    Ok(())
}

fn hotkey_backend(guest: &mut GuestControlClient) -> Result<String> {
    let outcome = wait_for_command(
        guest,
        command("/usr/bin/grep", &["HOTKEY_BACKEND", TRACE_LOG_PATH]),
        ACTION_TIMEOUT,
        |outcome| {
            outcome
                .stdout
                .lines()
                .any(|line| line.contains(" backend="))
        },
        "the hotkey backend trace",
    )?;
    let line = outcome
        .stdout
        .lines()
        .rev()
        .find(|line| line.contains(" HOTKEY_BACKEND "))
        .context("hotkey backend trace was missing")?;
    if line.contains("backend=native") {
        Ok("native".to_string())
    } else if line.contains("backend=global-hotkey") {
        Ok("global-hotkey-fallback".to_string())
    } else {
        bail!("unknown hotkey backend trace: {line}")
    }
}

fn key(qmp: &mut qmp::QmpClient, chord: &[&str]) -> Result<()> {
    qmp.send_keys(
        &chord
            .iter()
            .map(|key| (*key).to_string())
            .collect::<Vec<_>>(),
    )?;
    thread::sleep(KEY_SETTLE);
    Ok(())
}

fn dispatch_count(guest: &mut GuestControlClient, required: &str) -> Result<usize> {
    let outcome = require_exec(
        guest,
        command(
            "/usr/bin/python3",
            &[
                "-c",
                PROBE_COUNT_SCRIPT,
                TRACE_LOG_PATH,
                "HOTKEY_DISPATCH",
                required,
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    parse_count(&outcome)
}

fn wait_for_dispatch_count(
    guest: &mut GuestControlClient,
    required: &str,
    expected: usize,
) -> Result<usize> {
    let outcome = wait_for_command(
        guest,
        command(
            "/usr/bin/python3",
            &[
                "-c",
                PROBE_COUNT_SCRIPT,
                TRACE_LOG_PATH,
                "HOTKEY_DISPATCH",
                required,
            ],
        ),
        ACTION_TIMEOUT,
        |outcome| parse_count(outcome).is_ok_and(|count| count >= expected),
        &format!("{expected} hotkey dispatch probes"),
    )?;
    parse_count(&outcome)
}

fn parse_count(outcome: &ProcessOutcome) -> Result<usize> {
    outcome
        .stdout
        .trim()
        .parse()
        .context("hotkey probe count was not numeric")
}

fn request(
    guest: &mut GuestControlClient,
    auth: &str,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> Result<HttpResponse> {
    let url = format!("{}{path}", local_base_url());
    let mut args = vec![
        "--silent",
        "--show-error",
        "--write-out",
        "\n%{http_code}",
        "--header",
        auth,
        "--request",
        method,
    ];
    if let Some(body) = body {
        args.extend(["--header", "Content-Type: application/json", "--data", body]);
    }
    args.push(&url);
    let outcome = require_exec(guest, command("/usr/bin/curl", &args), COMMAND_TIMEOUT)?;
    let (body, status) = outcome
        .stdout
        .rsplit_once('\n')
        .context("curl response was missing an HTTP status")?;
    Ok(HttpResponse {
        status: status.parse().context("curl status was not numeric")?,
        body: body.to_string(),
    })
}

fn hotkey_config(id: &str, key: &str, action: &str, enabled: bool) -> String {
    serde_json::json!({
        "hotkeys": [binding(id, key, action, enabled)]
    })
    .to_string()
}

fn binding(id: &str, key: &str, action: &str, enabled: bool) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "key": key,
        "plugin_uid": LAUNCHER_UID,
        "action": action,
        "enabled": enabled,
    })
}

fn require_json_equal(expected: &str, actual: &str, context: &str) -> Result<()> {
    let expected: serde_json::Value = serde_json::from_str(expected)
        .with_context(|| format!("{context} expected invalid JSON"))?;
    let actual: serde_json::Value =
        serde_json::from_str(actual).with_context(|| format!("{context} actual invalid JSON"))?;
    if actual != expected {
        bail!("{context} mismatch: expected {expected}, got {actual}");
    }
    Ok(())
}

fn require_status(response: HttpResponse, expected: u16) -> Result<String> {
    if response.status != expected {
        bail!(
            "expected HTTP {expected}, got {}: {}",
            response.status,
            response.body
        );
    }
    Ok(response.body)
}

fn tray_pid(guest: &mut GuestControlClient) -> Result<String> {
    let outcome = require_exec(
        guest,
        command("/usr/bin/ps", &["-C", "qol-tray", "-o", "pid=,stat="]),
        COMMAND_TIMEOUT,
    )?;
    outcome
        .stdout
        .lines()
        .find_map(|line| {
            let mut fields = line.split_ascii_whitespace();
            let pid = fields.next()?;
            let state = fields.next()?;
            (!state.starts_with('Z')).then(|| pid.to_string())
        })
        .context("qol-tray was not running")
}

fn tray_fd_count(guest: &mut GuestControlClient) -> Result<usize> {
    let pid = tray_pid(guest)?;
    let path = format!("/proc/{pid}/fd");
    let outcome = require_exec(
        guest,
        command(
            "/usr/bin/find",
            &[&path, "-mindepth", "1", "-maxdepth", "1"],
        ),
        COMMAND_TIMEOUT,
    )?;
    Ok(outcome.stdout.lines().count())
}

#[cfg(test)]
mod tests {
    use super::parse_caps_lock;

    #[test]
    fn caps_lock_parser_handles_xset_spacing_and_missing_state() {
        let cases = [
            ("00: Caps Lock:   off    Num Lock: on", Some(false)),
            ("Caps Lock: on", Some(true)),
            ("Caps Lock:\t off", Some(false)),
            ("Num Lock: on", None),
            ("", None),
        ];

        for (input, expected) in cases {
            assert_eq!(parse_caps_lock(input), expected, "input: {input:?}");
        }
    }
}
