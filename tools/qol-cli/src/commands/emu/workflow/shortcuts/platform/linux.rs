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
    wait_for_probe_fields,
};
use super::Verdict;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const ACTION_TIMEOUT: Duration = Duration::from_secs(20);
const RACE_COUNT: usize = 40;
const RACE_PREFIX: &str = "shortcut-storm-race-";
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
    test_http_guards(&mut guest, &auth)?;
    let fds_before = tray_fd_count(&mut guest)?;
    test_concurrent_creates(&mut guest, &auth)?;
    let fds_after = tray_fd_count(&mut guest)?;
    require_fd_budget(fds_before, fds_after)?;
    let auth = restart_and_require_persistence(&mut guest, &auth)?;
    remove_race_shortcuts(&mut guest, &auth)?;
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
        "lifecycle, validation, guards, 40-way persistence race, restart, and cleanup passed",
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

fn test_concurrent_creates(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let token = auth
        .strip_prefix("X-Qol-Token: ")
        .context("tray auth header had an unexpected shape")?;
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
