use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use qol_conventions::{local_base_url, DEFAULT_PORT, TRACE_LOG_PATH};
use qol_dev_guest::{GuestControlClient, ProcessOutcome};

use crate::commands::emu::BootedVm;
use crate::progress::{step_label, StepKind};

use super::desktop::{
    command, connect_desktop_guest, current_trace_cursor, exec, install_payload,
    launch_tray_and_wait_api, require_exec, start_tray_and_wait_api, wait_for_command,
    wait_for_probe_fields, TRAY_BINARY,
};
use super::Verdict;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const ACTION_TIMEOUT: Duration = Duration::from_secs(20);
const RACE_COUNT: usize = 40;
const RACE_PREFIX: &str = "shortcut-storm-race-";
const APPS_DIR: &str = "/home/qol/.local/share/applications";
const LAUNCHER_ID: &str = "shortcut-storm-launcher";
const GATE_ID: &str = "shortcut-storm-gate";
const FORGED_ID: &str = "shortcut-storm-forged";
const PORT_CLOSED_SCRIPT: &str = r#"
import socket
import sys

socket_handle = socket.socket()
socket_handle.settimeout(0.5)
result = socket_handle.connect_ex(("127.0.0.1", int(sys.argv[1])))
socket_handle.close()
raise SystemExit(0 if result != 0 else 1)
"#;
const LINUX_CONCURRENT_CREATE_SCRIPT: &str = r#"
import concurrent.futures
import json
import sys
import urllib.error
import urllib.request

base = sys.argv[1]
token = sys.argv[2]
count = int(sys.argv[3])
prefix = sys.argv[4]

def create(index):
    payload = json.dumps({
        "id": f"{prefix}{index}",
        "name": f"Race {index}",
        "enabled": True,
        "export_to_launcher": False,
        "action": {
            "type": "launch_app",
            "app": {"type": "name", "name": "xed"},
        },
    }).encode()
    request = urllib.request.Request(
        base + "/api/shortcuts",
        data=payload,
        headers={"Content-Type": "application/json", "X-Qol-Token": token},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=10) as response:
            response.read()
            return response.status
    except urllib.error.HTTPError as error:
        error.read()
        return error.code

with concurrent.futures.ThreadPoolExecutor(max_workers=count) as pool:
    statuses = list(pool.map(create, range(count)))

request = urllib.request.Request(
    base + "/api/shortcuts",
    headers={"X-Qol-Token": token},
)
with urllib.request.urlopen(request, timeout=10) as response:
    shortcuts = json.load(response)["shortcuts"]

ids = [item["id"] for item in shortcuts if item["id"].startswith(prefix)]
print(json.dumps({
    "success": sum(200 <= status < 300 for status in statuses),
    "persisted": len(ids),
    "unique": len(set(ids)),
}))
"#;

const OVERSIZE_URL_BYTES: usize = 1024 * 1024 + 1024;
const LINUX_OVERSIZE_SCRIPT: &str = r#"
import json
import sys
import urllib.error
import urllib.request

base = sys.argv[1]
token = sys.argv[2]
size = int(sys.argv[3])

payload = json.dumps({
    "id": "shortcut-storm-oversize",
    "name": "Oversize",
    "action": {"type": "open_url", "url": "https://x.io/" + "a" * size},
}).encode()
request = urllib.request.Request(
    base + "/api/shortcuts",
    data=payload,
    headers={"Content-Type": "application/json", "X-Qol-Token": token},
    method="POST",
)
try:
    with urllib.request.urlopen(request, timeout=10) as response:
        response.read()
        print(response.status)
except urllib.error.HTTPError as error:
    error.read()
    print(error.code)
"#;

struct HttpResponse {
    status: u16,
    body: String,
}

pub(super) fn run(vm: &BootedVm) -> Result<Verdict> {
    let mut guest = connect_desktop_guest(vm)?;
    install_payload(&mut guest)?;
    let auth = start_tray_and_wait_api(&mut guest)?;
    let baseline = shortcut_ids(&mut guest, &auth)?;

    test_valid_lifecycle(&mut guest, &auth)?;
    test_invalid_requests(&mut guest, &auth)?;
    test_rejections_leave_state_untouched(&mut guest, &auth)?;
    test_http_guards(&mut guest, &auth)?;
    test_launcher_export(&mut guest, &auth)?;
    let fds_before = tray_fd_count(&mut guest)?;
    test_concurrent_creates(&mut guest, &auth)?;
    let fds_after = tray_fd_count(&mut guest)?;
    require_fd_budget(fds_before, fds_after)?;
    let auth = restart_and_require_persistence(&mut guest, &auth)?;
    remove_race_shortcuts(&mut guest, &auth)?;
    test_client_supplied_source(&mut guest, &auth)?;
    require_shortcut_ids(&mut guest, &auth, &baseline)?;

    let artifacts_dir = vm.run_dir.join("artifacts");
    std::fs::create_dir_all(&artifacts_dir)
        .with_context(|| format!("failed to create {}", artifacts_dir.display()))?;
    let final_config = artifacts_dir.join("shortcuts-final.json");
    let response = request(&mut guest, &auth, "GET", "/api/shortcuts", None)?;
    std::fs::write(&final_config, &response.body)
        .with_context(|| format!("failed to write {}", final_config.display()))?;
    let probes = require_exec(
        &mut guest,
        command("/usr/bin/grep", &["SHORTCUT_", TRACE_LOG_PATH]),
        COMMAND_TIMEOUT,
    )?;
    step_label(
        "storm",
        StepKind::Success,
        "lifecycle, validation, guards, launcher export, 40-way persistence race, restart, and cleanup passed",
    );
    Ok(Verdict {
        pass: true,
        traces: probes.stdout.lines().map(str::to_string).collect(),
        artifacts: vec![final_config],
    })
}

fn test_valid_lifecycle(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    create_and_launch_shortcut(guest, auth)?;
    disable_and_reject_shortcut(guest, auth)?;
    let _ = exec(
        guest,
        command("/usr/bin/pkill", &["--exact", "xed"]),
        COMMAND_TIMEOUT,
    )?;
    Ok(())
}

fn create_and_launch_shortcut(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let enabled = shortcut_payload("shortcut-storm-xed", true);
    require_status(
        request(guest, auth, "POST", "/api/shortcuts", Some(&enabled))?,
        200,
    )?;
    let cursor = current_trace_cursor(guest)?;
    require_status(
        request(
            guest,
            auth,
            "POST",
            "/api/shortcuts/shortcut-storm-xed/run",
            None,
        )?,
        200,
    )?;
    wait_for_probe_fields(
        guest,
        cursor,
        "SHORTCUT_OK",
        &["id=shortcut-storm-xed", "action=launch_app"],
        ACTION_TIMEOUT,
    )?;
    wait_for_command(
        guest,
        command("/usr/bin/pgrep", &["--exact", "xed"]),
        ACTION_TIMEOUT,
        |outcome| !outcome.stdout.trim().is_empty(),
        "Xed to launch from a shortcut",
    )?;
    Ok(())
}

fn disable_and_reject_shortcut(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let disabled = shortcut_payload("shortcut-storm-xed", false);
    require_status(
        request(
            guest,
            auth,
            "PUT",
            "/api/shortcuts/shortcut-storm-xed",
            Some(&disabled),
        )?,
        200,
    )?;
    let cursor = current_trace_cursor(guest)?;
    require_status(
        request(
            guest,
            auth,
            "POST",
            "/api/shortcuts/shortcut-storm-xed/run",
            None,
        )?,
        500,
    )?;
    wait_for_probe_fields(
        guest,
        cursor,
        "SHORTCUT_SKIP",
        &["id=shortcut-storm-xed", "reason=disabled"],
        ACTION_TIMEOUT,
    )?;
    require_status(
        request(
            guest,
            auth,
            "DELETE",
            "/api/shortcuts/shortcut-storm-xed",
            None,
        )?,
        200,
    )?;
    Ok(())
}

fn test_invalid_requests(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let cases = [
        r#"{"id":"bad path","name":"Bad","action":{"type":"launch_app","app":{"type":"name","name":"xed"}}}"#,
        r#"{"id":"bad-scheme","name":"Bad","action":{"type":"open_url","url":"file:///etc/passwd"}}"#,
        r#"{"id":"bad-traversal","name":"Bad","action":{"type":"launch_app","app":{"type":"path","path":"../bin/xed"}}}"#,
    ];
    for payload in cases {
        require_status(
            request(guest, auth, "POST", "/api/shortcuts", Some(payload))?,
            400,
        )?;
    }
    let payload = shortcut_payload("shortcut-storm-mismatch", true);
    require_status(
        request(
            guest,
            auth,
            "PUT",
            "/api/shortcuts/different-id",
            Some(&payload),
        )?,
        400,
    )?;
    require_status(
        request(
            guest,
            auth,
            "POST",
            "/api/shortcuts/shortcut-storm-missing/run",
            None,
        )?,
        404,
    )
    .map(|_| ())
}

fn test_http_guards(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let url = format!("{}/api/shortcuts", local_base_url());
    let missing = exec(
        guest,
        command(
            "/usr/bin/curl",
            &[
                "--silent",
                "--output",
                "/dev/null",
                "--write-out",
                "%{http_code}",
                &url,
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    require_status_output(&missing, 401)?;
    let hostile = exec(
        guest,
        command(
            "/usr/bin/curl",
            &[
                "--silent",
                "--output",
                "/dev/null",
                "--write-out",
                "%{http_code}",
                "--header",
                auth,
                "--header",
                "Host: hostile.invalid",
                &url,
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    require_status_output(&hostile, 403)
}

fn test_rejections_leave_state_untouched(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let before = shortcut_ids(guest, auth)?;
    let held = shortcut_payload("shortcut-storm-held", true);
    require_status(
        request(guest, auth, "POST", "/api/shortcuts", Some(&held))?,
        200,
    )?;
    let rejected = [
        (held.clone(), 400, "duplicate id"),
        (
            r#"{"id":"shortcut-storm-unknown","name":"X","action":{"type":"teleport"}}"#
                .to_string(),
            400,
            "unknown action kind",
        ),
        (
            r#"{"id":"shortcut-storm-broken","name":"X""#.to_string(),
            400,
            "truncated json",
        ),
        (
            r#"{"name":"X","action":{"type":"launch_app","app":{"type":"name","name":"xed"}}}"#
                .to_string(),
            400,
            "missing id",
        ),
    ];
    for (payload, expected, label) in rejected {
        let response = request(guest, auth, "POST", "/api/shortcuts", Some(&payload))?;
        if response.status != expected {
            bail!(
                "{label}: expected HTTP {expected}, got {}: {}",
                response.status,
                response.body
            );
        }
    }
    require_oversize_body_rejected(guest, auth)?;
    require_status(
        request(
            guest,
            auth,
            "DELETE",
            "/api/shortcuts/shortcut-storm-held",
            None,
        )?,
        200,
    )?;
    require_shortcut_ids(guest, auth, &before)
}

fn require_oversize_body_rejected(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let token = auth_token(auth)?;
    let outcome = require_exec(
        guest,
        command(
            "/usr/bin/python3",
            &[
                "-c",
                LINUX_OVERSIZE_SCRIPT,
                &local_base_url(),
                token,
                &OVERSIZE_URL_BYTES.to_string(),
            ],
        ),
        ACTION_TIMEOUT,
    )?;
    let status: u16 = outcome
        .stdout
        .trim()
        .parse()
        .context("oversize probe did not print an HTTP status")?;
    if status != 413 {
        bail!("oversize body: expected HTTP 413, got {status}");
    }
    Ok(())
}

fn test_launcher_export(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let entry = desktop_entry_name(LAUNCHER_ID);
    let path = format!("{APPS_DIR}/{entry}");
    require_status(
        request(
            guest,
            auth,
            "POST",
            "/api/shortcuts",
            Some(&shortcut_payload(LAUNCHER_ID, true)),
        )?,
        200,
    )?;
    wait_for_desktop_entry(guest, &entry, true)?;
    require_launcher_exec_line(guest, &path)?;
    run_shortcut_through_launcher_entry(guest)?;
    require_status(
        request(
            guest,
            auth,
            "PUT",
            &format!("/api/shortcuts/{LAUNCHER_ID}"),
            Some(&shortcut_payload(LAUNCHER_ID, false)),
        )?,
        200,
    )?;
    wait_for_desktop_entry(guest, &entry, false)?;
    require_status(
        request(
            guest,
            auth,
            "PUT",
            &format!("/api/shortcuts/{LAUNCHER_ID}"),
            Some(&shortcut_payload(LAUNCHER_ID, true)),
        )?,
        200,
    )?;
    wait_for_desktop_entry(guest, &entry, true)?;
    require_status(
        request(
            guest,
            auth,
            "DELETE",
            &format!("/api/shortcuts/{LAUNCHER_ID}"),
            None,
        )?,
        200,
    )?;
    wait_for_desktop_entry(guest, &entry, false)
}

fn run_shortcut_through_launcher_entry(guest: &mut GuestControlClient) -> Result<()> {
    let cursor = current_trace_cursor(guest)?;
    require_exec(
        guest,
        command(TRAY_BINARY, &["exec", "shortcut", LAUNCHER_ID]),
        ACTION_TIMEOUT,
    )?;
    wait_for_probe_fields(
        guest,
        cursor,
        "SHORTCUT_OK",
        &[&format!("id={LAUNCHER_ID}"), "action=launch_app"],
        ACTION_TIMEOUT,
    )?;
    wait_for_command(
        guest,
        command("/usr/bin/pgrep", &["--exact", "xed"]),
        ACTION_TIMEOUT,
        |outcome| !outcome.stdout.trim().is_empty(),
        "Xed to launch from the exported launcher entry",
    )?;
    let _ = exec(
        guest,
        command("/usr/bin/pkill", &["--exact", "xed"]),
        COMMAND_TIMEOUT,
    )?;
    let missing = exec(
        guest,
        command(TRAY_BINARY, &["exec", "shortcut", "shortcut-storm-missing"]),
        COMMAND_TIMEOUT,
    )?;
    if missing.exit_code != Some(1) {
        bail!(
            "`qol-tray exec shortcut` on a missing id exited {:?}: {}",
            missing.exit_code,
            missing.stderr.trim()
        );
    }
    Ok(())
}

fn require_launcher_exec_line(guest: &mut GuestControlClient, path: &str) -> Result<()> {
    let outcome = require_exec(guest, command("/usr/bin/cat", &[path]), COMMAND_TIMEOUT)?;
    let exec_line = outcome
        .stdout
        .lines()
        .find(|line| line.starts_with("Exec="))
        .with_context(|| format!("{path} has no Exec line"))?
        .to_string();
    for fragment in [
        TRAY_BINARY,
        "\"exec\"",
        "\"shortcut\"",
        &format!("\"{LAUNCHER_ID}\""),
    ] {
        if !exec_line.contains(fragment) {
            bail!("launcher entry Exec line is missing {fragment}: {exec_line}");
        }
    }
    Ok(())
}

fn test_client_supplied_source(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let response = request(
        guest,
        auth,
        "POST",
        "/api/shortcuts",
        Some(&forged_source_payload()),
    )?;
    if response.status == 400 {
        return Ok(());
    }
    require_status(response, 200)?;
    require_launcher_sync_settled(guest, auth)?;
    let ids = shortcut_ids(guest, auth)?;
    if !ids.contains(FORGED_ID) {
        bail!(
            "POST /api/shortcuts answered 200 for `{FORGED_ID}`, then plugin reconciliation deleted it before the next read"
        );
    }
    require_status(
        request(
            guest,
            auth,
            "DELETE",
            &format!("/api/shortcuts/{FORGED_ID}"),
            None,
        )?,
        200,
    )?;
    Ok(())
}

fn require_launcher_sync_settled(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let entry = desktop_entry_name(GATE_ID);
    require_status(
        request(
            guest,
            auth,
            "POST",
            "/api/shortcuts",
            Some(&shortcut_payload(GATE_ID, true)),
        )?,
        200,
    )?;
    wait_for_desktop_entry(guest, &entry, true)?;
    require_status(
        request(
            guest,
            auth,
            "DELETE",
            &format!("/api/shortcuts/{GATE_ID}"),
            None,
        )?,
        200,
    )?;
    wait_for_desktop_entry(guest, &entry, false)
}

fn wait_for_desktop_entry(
    guest: &mut GuestControlClient,
    entry: &str,
    expected: bool,
) -> Result<()> {
    let description = match expected {
        true => format!("launcher entry {entry} to be written"),
        false => format!("launcher entry {entry} to be removed"),
    };
    wait_for_command(
        guest,
        command(
            "/usr/bin/find",
            &[APPS_DIR, "-maxdepth", "1", "-name", entry],
        ),
        ACTION_TIMEOUT,
        |outcome| !outcome.stdout.trim().is_empty() == expected,
        &description,
    )?;
    Ok(())
}

fn desktop_entry_name(id: &str) -> String {
    format!("qol-shortcut-{id}.desktop")
}

fn test_concurrent_creates(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let token = auth_token(auth)?;
    let count = RACE_COUNT.to_string();
    let outcome = require_exec(
        guest,
        command(
            "/usr/bin/python3",
            &[
                "-c",
                LINUX_CONCURRENT_CREATE_SCRIPT,
                &local_base_url(),
                token,
                &count,
                RACE_PREFIX,
            ],
        ),
        ACTION_TIMEOUT,
    )?;
    let summary: serde_json::Value =
        serde_json::from_str(outcome.stdout.trim()).context("invalid race summary")?;
    for field in ["success", "persisted", "unique"] {
        if summary[field].as_u64() != Some(RACE_COUNT as u64) {
            bail!("shortcut race lost writes: {}", outcome.stdout.trim());
        }
    }
    Ok(())
}

fn restart_and_require_persistence(guest: &mut GuestControlClient, auth: &str) -> Result<String> {
    let before = tray_pid(guest)?;
    require_status(request(guest, auth, "POST", "/api/shutdown", None)?, 202)?;
    let port = DEFAULT_PORT.to_string();
    wait_for_command(
        guest,
        command("/usr/bin/python3", &["-c", PORT_CLOSED_SCRIPT, &port]),
        ACTION_TIMEOUT,
        |_| true,
        "the tray HTTP port to close after authenticated shutdown",
    )?;
    let auth = launch_tray_and_wait_api(guest)?;
    let after = tray_pid(guest)?;
    if before == after {
        bail!("qol-tray did not restart during shortcut persistence test");
    }
    let ids = shortcut_ids(guest, &auth)?;
    let persisted = ids.iter().filter(|id| id.starts_with(RACE_PREFIX)).count();
    if persisted != RACE_COUNT {
        bail!("restart retained {persisted}/{RACE_COUNT} race shortcuts");
    }
    Ok(auth)
}

fn remove_race_shortcuts(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    for index in 0..RACE_COUNT {
        let path = format!("/api/shortcuts/{RACE_PREFIX}{index}");
        require_status(request(guest, auth, "DELETE", &path, None)?, 200)?;
    }
    Ok(())
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

fn shortcut_payload(id: &str, enabled: bool) -> String {
    serde_json::json!({
        "id": id,
        "name": "Shortcut Storm Xed",
        "enabled": enabled,
        "export_to_launcher": true,
        "action": {
            "type": "launch_app",
            "app": {"type": "name", "name": "xed"}
        }
    })
    .to_string()
}

fn forged_source_payload() -> String {
    serde_json::json!({
        "id": FORGED_ID,
        "name": "Forged Source",
        "enabled": true,
        "export_to_launcher": false,
        "source": {
            "type": "plugin_manifest",
            "plugin_id": "plugin-ghost",
            "shortcut_id": "open"
        },
        "action": {
            "type": "launch_app",
            "app": {"type": "name", "name": "xed"}
        }
    })
    .to_string()
}

fn auth_token(auth: &str) -> Result<&str> {
    auth.strip_prefix("X-Qol-Token: ")
        .context("tray auth header had an unexpected shape")
}

fn shortcut_ids(guest: &mut GuestControlClient, auth: &str) -> Result<BTreeSet<String>> {
    let response = request(guest, auth, "GET", "/api/shortcuts", None)?;
    require_status(response, 200).and_then(|body| {
        let config: serde_json::Value =
            serde_json::from_str(&body).context("shortcut list was invalid JSON")?;
        let shortcuts = config["shortcuts"]
            .as_array()
            .context("shortcut list had no shortcuts array")?;
        Ok(shortcuts
            .iter()
            .filter_map(|item| item["id"].as_str().map(str::to_string))
            .collect())
    })
}

fn require_shortcut_ids(
    guest: &mut GuestControlClient,
    auth: &str,
    expected: &BTreeSet<String>,
) -> Result<()> {
    let actual = shortcut_ids(guest, auth)?;
    if actual != *expected {
        bail!("shortcut cleanup mismatch: expected {expected:?}, got {actual:?}");
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

fn require_status_output(outcome: &ProcessOutcome, expected: u16) -> Result<()> {
    let status = outcome
        .stdout
        .trim()
        .parse::<u16>()
        .context("curl guard status was not numeric")?;
    if status != expected {
        bail!("expected HTTP {expected}, got {status}");
    }
    Ok(())
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

fn require_fd_budget(before: usize, after: usize) -> Result<()> {
    if after > before.saturating_add(8) {
        bail!("qol-tray file descriptors grew from {before} to {after}");
    }
    Ok(())
}
