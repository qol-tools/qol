use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use qol_conventions::{local_base_url, DEFAULT_PORT, TRACE_LOG_PATH};
use qol_dev_guest::GuestControlClient;

use crate::commands::emu::{qmp, BootedVm};
use crate::progress::{step_label, StepKind};

use super::desktop::{
    command, connect_desktop_guest, install_payload, launch_tray_and_wait_api, require_exec,
    start_tray_and_wait_plugin, wait_for_command,
};
use super::Verdict;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const ACTION_TIMEOUT: Duration = Duration::from_secs(25);
const KEY_SETTLE: Duration = Duration::from_millis(200);
const LAUNCHER_UID: &str = "5cc75f62-2e3b-463c-ac7b-ae269cff1ef1";
const QOL_COMBO: &str = "Ctrl+Alt+F9";
const GTK_COMBO: &str = "['<Primary><Alt>F9']";
const KEYBINDINGS: &str = "/org/cinnamon/desktop/keybindings";
const MANAGED_KEY: &str = "/org/cinnamon/desktop/keybindings/custom-keybindings/custom9/binding";
const ORPHAN_KEY: &str = "/org/cinnamon/desktop/keybindings/custom8/binding";
const DESKLETS_KEY: &str = "/org/cinnamon/desktop/keybindings/show-desklets";
const SUBSET_COMBO: &str = "Shift+Super+S";
const CLAIMS_GLOB: &str = "*/host-takeover/qol-tray-hotkeys/takeover-*";
const CLEARED: &str = "@as []";
const PORT_CLOSED_SCRIPT: &str = r#"
import socket
import sys

handle = socket.socket()
handle.settimeout(0.5)
result = handle.connect_ex(("127.0.0.1", int(sys.argv[1])))
handle.close()
raise SystemExit(0 if result != 0 else 1)
"#;

struct HttpResponse {
    status: u16,
    body: String,
}

pub(super) fn run(vm: &BootedVm) -> Result<Verdict> {
    let mut guest = connect_desktop_guest(vm)?;
    install_payload(&mut guest)?;
    seed_conflicts(&mut guest)?;

    let mut auth = start_tray_and_wait_plugin(&mut guest, "plugin-launcher")?;
    let baseline = require_status(
        request(&mut guest, &auth, "GET", "/api/hotkeys", None)?,
        200,
    )?;
    set_hotkeys(&mut guest, &auth, &hotkey_config())?;
    require_binding(&mut guest, MANAGED_KEY, GTK_COMBO, "before the doctor pass")?;

    restart_tray(&mut guest, &mut auth)?;
    require_taken_over(&mut guest)?;

    let mut qmp = qmp::connect_verified(vm.qmp_port, COMMAND_TIMEOUT, &vm.run_id)?;
    super::super::super::hotkeys::require_passthrough(&mut guest, &mut qmp)?;
    require_chord_reaches_qol(&mut guest, &mut qmp)?;
    require_subset_chord_reaches_qol(&mut guest, &mut qmp)?;

    set_hotkeys(&mut guest, &auth, &baseline)?;
    shutdown_tray(&mut guest, &auth)?;
    require_reconciled(&mut guest, "after qol-tray exited")?;

    auth = launch_tray_and_wait_api(&mut guest)?;
    require_reconciled(&mut guest, "after qol-tray restarted")?;
    shutdown_tray(&mut guest, &auth)?;
    require_reconciled(&mut guest, "after the second shutdown")?;

    let artifacts = write_artifacts(vm, &mut guest)?;
    step_label(
        "shadow",
        StepKind::Success,
        "managed shortcuts were restored while orphan cleanup stayed quarantined",
    );
    Ok(Verdict {
        pass: true,
        traces: vec![
            format!("{MANAGED_KEY} taken over and restored"),
            format!("{ORPHAN_KEY} cleared across a tray restart"),
            format!("{DESKLETS_KEY} cleared and restored across the tray lifecycle"),
        ],
        artifacts,
    })
}

fn seed_conflicts(guest: &mut GuestControlClient) -> Result<()> {
    let custom_list = format!("{KEYBINDINGS}/custom-list");
    let writes: [(&str, &str); 6] = [
        (MANAGED_KEY, GTK_COMBO),
        (
            "/org/cinnamon/desktop/keybindings/custom-keybindings/custom9/command",
            "'true'",
        ),
        (
            "/org/cinnamon/desktop/keybindings/custom-keybindings/custom9/name",
            "'qol shadow probe'",
        ),
        (custom_list.as_str(), "['custom9']"),
        (ORPHAN_KEY, GTK_COMBO),
        (
            "/org/cinnamon/desktop/keybindings/custom8/command",
            "'true'",
        ),
    ];
    for (key, value) in writes {
        dconf_write(guest, key, value)?;
    }
    dconf_reset(guest, DESKLETS_KEY)?;
    step_label(
        "seed",
        StepKind::Success,
        "a schema-backed, an orphaned legacy, and an unset show-desklets default all conflict with qol hotkeys",
    );
    Ok(())
}

fn require_taken_over(guest: &mut GuestControlClient) -> Result<()> {
    require_binding(guest, MANAGED_KEY, CLEARED, "after the doctor pass")?;
    require_binding(guest, ORPHAN_KEY, CLEARED, "after the doctor pass")?;
    require_binding(guest, DESKLETS_KEY, CLEARED, "after the doctor pass")?;
    let markers = claim_markers(guest)?;
    if markers.len() != 3 {
        bail!("expected 3 takeover claims to be recorded, found {markers:?}");
    }
    let mut exact_claims = 0;
    let mut subset_claims = 0;
    for marker in &markers {
        let body = require_exec(guest, command("/usr/bin/cat", &[marker]), COMMAND_TIMEOUT)?;
        if body.stdout.contains(QOL_COMBO) && body.stdout.contains("<Primary><Alt>F9") {
            exact_claims += 1;
        } else if body.stdout.contains(SUBSET_COMBO) && body.stdout.contains("previous_unset") {
            subset_claims += 1;
        } else {
            bail!("claim {marker} carries no expected combo and restore value: {body:?}");
        }
    }
    if exact_claims != 2 || subset_claims != 1 {
        bail!(
            "expected 2 exact and 1 subset claim, found {exact_claims} exact and {subset_claims} subset"
        );
    }
    step_label(
        "takeover",
        StepKind::Success,
        "all three shortcuts were cleared and their previous values recorded",
    );
    Ok(())
}

fn require_chord_opens_launcher(
    guest: &mut GuestControlClient,
    qmp: &mut qmp::QmpClient,
    keys: &[&str],
    what: &str,
) -> Result<()> {
    let mut last_error = None;
    for _attempt in 0..3 {
        let chord: Vec<String> = keys.iter().map(|key| key.to_string()).collect();
        qmp.send_keys(&chord)?;
        thread::sleep(KEY_SETTLE);
        match wait_for_command(
            guest,
            command("/usr/bin/xdotool", &["search", "--name", "^qol-launcher@"]),
            ACTION_TIMEOUT,
            |outcome| !outcome.stdout.trim().is_empty(),
            &format!("the launcher window opened by the {what} chord"),
        ) {
            Ok(_) => {
                qmp.send_keys(&["esc".into()])?;
                thread::sleep(KEY_SETTLE);
                return Ok(());
            }
            Err(error) => {
                last_error = Some(error);
                let _ = qmp.send_keys(&["esc".into()]);
                thread::sleep(Duration::from_secs(3));
            }
        }
    }
    Err(anyhow::anyhow!(
        "the {what} chord never opened the launcher{}",
        last_error
            .map(|error| format!(": {error:#}"))
            .unwrap_or_default()
    ))
}

fn require_chord_reaches_qol(
    guest: &mut GuestControlClient,
    qmp: &mut qmp::QmpClient,
) -> Result<()> {
    require_chord_opens_launcher(guest, qmp, &["ctrl", "alt", "f9"], "reclaimed")
}

fn require_subset_chord_reaches_qol(
    guest: &mut GuestControlClient,
    qmp: &mut qmp::QmpClient,
) -> Result<()> {
    require_chord_opens_launcher(guest, qmp, &["shift", "meta_l", "s"], "subset-modifier")
}

fn require_reconciled(guest: &mut GuestControlClient, phase: &str) -> Result<()> {
    require_binding(guest, MANAGED_KEY, GTK_COMBO, phase)?;
    require_binding(guest, DESKLETS_KEY, "", phase)?;
    require_binding(guest, ORPHAN_KEY, CLEARED, phase)?;
    let markers = claim_markers(guest)?;
    if markers.len() != 1 {
        bail!("expected one quarantined orphan claim {phase}, found {markers:?}");
    }
    let body = require_exec(
        guest,
        command("/usr/bin/cat", &[&markers[0]]),
        COMMAND_TIMEOUT,
    )?;
    if !body.stdout.contains("legacy_orphan") || !body.stdout.contains(QOL_COMBO) {
        bail!("quarantined claim {phase} is not the orphaned shortcut: {body:?}");
    }
    step_label(
        "reconcile",
        StepKind::Success,
        "the managed shortcut was restored and the orphan remained cleared",
    );
    Ok(())
}

fn require_binding(
    guest: &mut GuestControlClient,
    key: &str,
    expected: &str,
    phase: &str,
) -> Result<()> {
    let outcome = require_exec(
        guest,
        command("/usr/bin/dconf", &["read", key]),
        COMMAND_TIMEOUT,
    )?;
    let actual = outcome.stdout.trim();
    if actual != expected {
        bail!("{key} {phase}: expected {expected}, got {actual:?}");
    }
    Ok(())
}

fn claim_markers(guest: &mut GuestControlClient) -> Result<Vec<String>> {
    let outcome = require_exec(
        guest,
        command(
            "/bin/sh",
            &[
                "-c",
                &format!("find /root /home -type f -path '{CLAIMS_GLOB}' 2>/dev/null || true"),
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    Ok(outcome
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

fn dconf_write(guest: &mut GuestControlClient, key: &str, value: &str) -> Result<()> {
    require_exec(
        guest,
        command("/usr/bin/dconf", &["write", key, value]),
        COMMAND_TIMEOUT,
    )?;
    Ok(())
}

fn dconf_reset(guest: &mut GuestControlClient, key: &str) -> Result<()> {
    require_exec(
        guest,
        command("/usr/bin/dconf", &["reset", key]),
        COMMAND_TIMEOUT,
    )?;
    Ok(())
}

fn restart_tray(guest: &mut GuestControlClient, auth: &mut String) -> Result<()> {
    shutdown_tray(guest, auth)?;
    *auth = launch_tray_and_wait_api(guest)?;
    Ok(())
}

fn shutdown_tray(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    require_status(request(guest, auth, "POST", "/api/shutdown", None)?, 202)?;
    let port = DEFAULT_PORT.to_string();
    wait_for_command(
        guest,
        command("/usr/bin/python3", &["-c", PORT_CLOSED_SCRIPT, &port]),
        ACTION_TIMEOUT,
        |_| true,
        "the tray HTTP port to close",
    )?;
    Ok(())
}

fn write_artifacts(
    vm: &BootedVm,
    guest: &mut GuestControlClient,
) -> Result<Vec<std::path::PathBuf>> {
    let dir = vm.run_dir.join("artifacts");
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    let dump = require_exec(
        guest,
        command(
            "/usr/bin/dconf",
            &["dump", "/org/cinnamon/desktop/keybindings/"],
        ),
        COMMAND_TIMEOUT,
    )?;
    let dump_path = dir.join("hotkey-shadow-keybindings.ini");
    std::fs::write(&dump_path, &dump.stdout)
        .with_context(|| format!("failed to write {}", dump_path.display()))?;
    let trace = require_exec(
        guest,
        command("/usr/bin/grep", &["HOTKEY_", TRACE_LOG_PATH]),
        COMMAND_TIMEOUT,
    )?;
    if !trace
        .stdout
        .lines()
        .any(|line| line.contains("HOTKEY_TAKEOVER") && line.contains("decision=quarantine"))
    {
        bail!("hotkey trace does not record the orphan quarantine decision");
    }
    let trace_path = dir.join("hotkey-shadow-trace.log");
    std::fs::write(&trace_path, &trace.stdout)
        .with_context(|| format!("failed to write {}", trace_path.display()))?;
    Ok(vec![dump_path, trace_path])
}

fn set_hotkeys(guest: &mut GuestControlClient, auth: &str, body: &str) -> Result<()> {
    require_status(
        request(guest, auth, "PUT", "/api/hotkeys", Some(body))?,
        200,
    )?;
    Ok(())
}

fn hotkey_config() -> String {
    serde_json::json!({
        "hotkeys": [
            {
                "id": "hotkey-shadow-probe",
                "key": QOL_COMBO,
                "plugin_uid": LAUNCHER_UID,
                "action": "open",
                "enabled": true,
            },
            {
                "id": "hotkey-shadow-subset-probe",
                "key": SUBSET_COMBO,
                "plugin_uid": LAUNCHER_UID,
                "action": "open",
                "enabled": true,
            },
        ]
    })
    .to_string()
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
