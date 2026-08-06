use anyhow::{bail, Context, Result};

use crate::commands::emu::workflow::hotkey_shadow_boot::platform::desktop::{
    command, connect_desktop_guest, current_trace_cursor, exec, install_payload,
    launch_tray_and_wait_api, require_exec, spawn, wait_for_command, wait_for_probe_line,
};
use crate::commands::emu::workflow::hotkey_shadow_boot::platform::Verdict;
use crate::commands::emu::{qmp, BootedVm};
use crate::progress::{step_label, StepKind};
use qol_conventions::{local_base_url, TRACE_LOG_PATH};
use qol_dev_guest::GuestControlClient;

use std::thread;
use std::time::Duration;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const BOOT_READY_TIMEOUT: Duration = Duration::from_secs(6 * 60);
const BOOT_SETTLE: Duration = Duration::from_secs(150);
const ACTION_TIMEOUT: Duration = Duration::from_secs(25);
const KEY_SETTLE: Duration = Duration::from_millis(200);
const LAUNCHER_UID: &str = "5cc75f62-2e3b-463c-ac7b-ae269cff1ef1";
const QOL_COMBO: &str = "Shift+Super+S";
const GTK_COMBO: &str = "['<Shift><Super>s']";
const MANAGED_KEY: &str = "/org/cinnamon/desktop/keybindings/custom-keybindings/custom9/binding";
const MANAGED_COMMAND: &str =
    "/org/cinnamon/desktop/keybindings/custom-keybindings/custom9/command";
const MANAGED_NAME: &str = "/org/cinnamon/desktop/keybindings/custom-keybindings/custom9/name";
const CUSTOM_LIST: &str = "/org/cinnamon/desktop/keybindings/custom-list";
const CLAIMS_GLOB: &str = "*/host-takeover/qol-tray-hotkeys/takeover-*";
const CLEARED: &str = "@as []";
const AUTOSTART_PATH: &str = "/home/qol/.config/autostart/qol-tray.desktop";
const DE_MARKER: &str = "/tmp/de-shadow-evidence";

pub(super) fn run(vm: &BootedVm) -> Result<Verdict> {
    let mut guest = connect_desktop_guest(vm)?;
    install_payload(&mut guest)?;
    stage_boot_scenario(&mut guest)?;
    reboot_guest_cleanly(&mut guest)?;
    drop(guest);
    step_label(
        "reboot",
        StepKind::Pending,
        "rebooting so the tray autostarts against the seeded desktop binding",
    );
    thread::sleep(BOOT_SETTLE);

    let mut guest = connect_after_reboot(vm)?;
    let mut auth = match wait_for_autostart_tray(&mut guest) {
        Ok(auth) => auth,
        Err(error) if error.to_string().contains("duplicate hello") => {
            drop(guest);
            guest = connect_after_reboot(vm)?;
            match wait_for_autostart_tray(&mut guest) {
                Ok(auth) => auth,
                Err(error) => return Err(boot_diagnostics(&mut guest, "", error)),
            }
        }
        Err(error) => return Err(boot_diagnostics(&mut guest, "", error)),
    };

    require_binding(
        &mut guest,
        MANAGED_KEY,
        CLEARED,
        "after the boot doctor pass",
    )
    .map_err(|error| boot_diagnostics(&mut guest, &auth, error))?;
    require_boot_claim(&mut guest)?;
    let mut qmp = qmp::connect_verified(vm.qmp_port, COMMAND_TIMEOUT, &vm.run_id)?;
    require_chord_reaches_qol(&mut guest, &mut qmp)
        .map_err(|error| boot_diagnostics(&mut guest, &auth, error))?;
    require_de_stays_quiet(&mut guest)
        .map_err(|error| boot_diagnostics(&mut guest, &auth, error))?;

    let artifacts = write_artifacts(vm, &mut guest, &auth)?;
    step_label(
        "shadow-boot",
        StepKind::Success,
        "the chord won on the fresh boot; the desktop binding stayed cleared and claimed",
    );
    Ok(Verdict {
        pass: true,
        traces: vec![
            format!("{MANAGED_KEY} taken over and claimed during the boot doctor pass"),
            "Shift+Super+S reached the autostarted tray on the fresh boot".to_string(),
        ],
        artifacts,
    })
}

fn stage_boot_scenario(guest: &mut GuestControlClient) -> Result<()> {
    stage_watcher(guest)?;
    let auth = launch_tray_and_wait_api(guest)?;
    set_hotkeys(guest, &auth, &hotkey_config())?;
    let reloaded = require_status(request(guest, &auth, "GET", "/api/hotkeys", None)?, 200)?;
    if !reloaded.contains("Shift+Super+S") {
        bail!("staged hotkey config did not round-trip: {reloaded}");
    }
    harvest_staging_tray_stderr(guest)?;
    seed_de_conflict(guest)?;
    write_autostart(guest)?;
    thread::sleep(Duration::from_secs(8));
    harvest_watch_transitions(guest)?;
    let pre_reboot = require_exec(
        guest,
        command(
            "/usr/bin/bash",
            &["-lc", "stat -c '%y %s' /home/qol/.config/qol-tray/profile/default/os/linux/hotkeys.json 2>&1"],
        ),
        COMMAND_TIMEOUT,
    )?;
    step_label(
        "pre-reboot",
        StepKind::Success,
        &format!("hotkeys file before reboot: {}", pre_reboot.stdout.trim()),
    );
    step_label(
        "stage",
        StepKind::Success,
        "hotkeys configured through the app API; desktop conflict and tray autostart staged",
    );
    Ok(())
}

fn stage_watcher(guest: &mut GuestControlClient) -> Result<()> {
    let script_b64 = "aW1wb3J0IG9zCmltcG9ydCB0aW1lCgpwYXRoID0gIi9ob21lL3FvbC8uY29uZmlnL3FvbC10cmF5L3Byb2ZpbGUvZGVmYXVsdC9vcy9saW51eC9ob3RrZXlzLmpzb24iCm91dCA9IG9wZW4oIi9ob21lL3FvbC9ob3RrZXlzLXdhdGNoLmxvZyIsICJhIikKbGFzdCA9IE5vbmUKd2hpbGUgVHJ1ZToKICAgIHRyeToKICAgICAgICBzdCA9IG9zLnN0YXQocGF0aCkKICAgICAgICBzaXplLCBtdGltZSA9IHN0LnN0X3NpemUsIGludChzdC5zdF9tdGltZSAqIDEwMDApCiAgICBleGNlcHQgT1NFcnJvcjoKICAgICAgICBzaXplLCBtdGltZSA9IC0xLCAwCiAgICBrZXkgPSAoc2l6ZSwgbXRpbWUpCiAgICBpZiBrZXkgIT0gbGFzdDoKICAgICAgICBvdXQud3JpdGUoIiVkICVkICVkXG4iICUgKGludCh0aW1lLnRpbWUoKSAqIDEwMDApLCBzaXplLCBtdGltZSkpCiAgICAgICAgb3V0LmZsdXNoKCkKICAgICAgICBsYXN0ID0ga2V5CiAgICB0aW1lLnNsZWVwKDAuMDEpCg==";
    let desktop = "[Desktop Entry]\nType=Application\nName=qol-hotkeys-watch\nExec=/usr/bin/python3 /home/qol/hotkeys-watch.py\nX-GNOME-Autostart-enabled=true\n";
    require_exec(
        guest,
        command(
            "/usr/bin/sh",
            &[
                "-c",
                &format!(
                    "echo {} | base64 -d > /home/qol/hotkeys-watch.py && printf '%s' '{}' > /home/qol/.config/autostart/00-hotkeys-watch.desktop",
                    script_b64, desktop
                ),
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    let pid = spawn(
        guest,
        command("/usr/bin/python3", &["/home/qol/hotkeys-watch.py"]),
    )?;
    thread::sleep(Duration::from_secs(2));
    let health = require_exec(
        guest,
        command(
            "/usr/bin/bash",
            &[
                "-lc",
                &format!(
                    "wc -l < /home/qol/hotkeys-watch.log 2>/dev/null || echo missing; echo ---stderr---; cat /tmp/qol-guest-runner/{pid}/stderr 2>/dev/null | tail -20",
                ),
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    step_label(
        "watcher",
        StepKind::Success,
        &format!("sampler health: {}", health.stdout.replace('\n', " | ")),
    );
    Ok(())
}

fn harvest_watch_transitions(guest: &mut GuestControlClient) -> Result<()> {
    let outcome = require_exec(
        guest,
        command(
            "/usr/bin/bash",
            &[
                "-lc",
                "cat /home/qol/hotkeys-watch.log 2>/dev/null | tail -40 || true",
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    step_label(
        "watch-transitions",
        StepKind::Success,
        &format!(
            "hotkeys file transitions: {}",
            outcome.stdout.replace('\n', " | ")
        ),
    );
    Ok(())
}

fn harvest_staging_tray_stderr(guest: &mut GuestControlClient) -> Result<()> {
    let outcome = require_exec(
        guest,
        command(
            "/usr/bin/bash",
            &[
                "-lc",
                "cat /tmp/qol-guest-runner/*/stderr 2>/dev/null | grep -E 'hotkey|shortcut|doctor|error|panic|save|load|material|config' | tail -25",
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    step_label(
        "staging-stderr",
        StepKind::Success,
        &format!(
            "staging tray stderr: {}",
            outcome.stdout.replace('\n', " | ")
        ),
    );
    Ok(())
}

fn connect_after_reboot(vm: &BootedVm) -> Result<GuestControlClient> {
    let deadline = std::time::Instant::now() + BOOT_READY_TIMEOUT;
    loop {
        match connect_desktop_guest(vm) {
            Ok(guest) => return Ok(guest),
            Err(error) => {
                if std::time::Instant::now() >= deadline {
                    return Err(error);
                }
                thread::sleep(Duration::from_secs(10));
            }
        }
    }
}

fn reboot_guest_cleanly(guest: &mut GuestControlClient) -> Result<()> {
    require_exec(
        guest,
        command("/usr/bin/systemctl", &["reboot"]),
        COMMAND_TIMEOUT,
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

fn hotkey_config() -> String {
    serde_json::json!({
        "hotkeys": [{
            "id": "hotkey-shadow-boot-probe",
            "key": QOL_COMBO,
            "plugin_uid": LAUNCHER_UID,
            "action": "open",
            "enabled": true,
        }]
    })
    .to_string()
}

fn seed_de_conflict(guest: &mut GuestControlClient) -> Result<()> {
    let writes: [(&str, &str); 4] = [
        (MANAGED_KEY, GTK_COMBO),
        (MANAGED_COMMAND, "'touch /tmp/de-shadow-evidence'"),
        (MANAGED_NAME, "'qol boot shadow probe'"),
        (CUSTOM_LIST, "['custom9']"),
    ];
    for (key, value) in writes {
        dconf_write(guest, key, value)?;
    }
    Ok(())
}

fn write_autostart(guest: &mut GuestControlClient) -> Result<()> {
    require_exec(
        guest,
        command(
            "/usr/bin/sh",
            &[
                "-c",
                &format!(
                    "install -d -m 0755 /home/qol/.config/autostart && printf '%s' '{}' > {AUTOSTART_PATH}",
                    autostart_desktop()
                ),
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
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

struct HttpResponse {
    status: u16,
    body: String,
}

fn autostart_desktop() -> &'static str {
    "[Desktop Entry]\nType=Application\nName=qol-tray\nExec=/home/qol/.local/bin/qol-tray\nX-GNOME-Autostart-enabled=true\n"
}

fn wait_for_autostart_tray(guest: &mut GuestControlClient) -> Result<String> {
    step_label(
        "tray",
        StepKind::Pending,
        "waiting for the autostarted production tray API",
    );
    let token = wait_for_command(
        guest,
        command("/usr/bin/cat", &["/home/qol/.config/qol-tray/.http-token"]),
        BOOT_READY_TIMEOUT,
        |outcome| !outcome.stdout.trim().is_empty(),
        "the autostarted tray HTTP token",
    )?
    .stdout
    .trim()
    .to_string();
    let auth = format!("X-Qol-Token: {token}");
    let api = format!("{}/api/shortcuts", local_base_url());
    wait_for_command(
        guest,
        command(
            "/usr/bin/curl",
            &["--fail", "--silent", "--header", &auth, &api],
        ),
        BOOT_READY_TIMEOUT,
        |_| true,
        "the autostarted tray shortcuts API",
    )?;
    Ok(auth)
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

fn require_boot_claim(guest: &mut GuestControlClient) -> Result<()> {
    let markers = claim_markers(guest)?;
    if markers.len() != 1 {
        bail!("expected one boot takeover claim, found {markers:?}");
    }
    let body = require_exec(
        guest,
        command("/usr/bin/cat", &[&markers[0]]),
        COMMAND_TIMEOUT,
    )?;
    if !body.stdout.contains(QOL_COMBO) || !body.stdout.contains(GTK_COMBO) {
        bail!("boot claim does not carry the combo and the value to restore: {body:?}");
    }
    step_label(
        "claim",
        StepKind::Success,
        "the boot doctor pass recorded the reversible takeover claim",
    );
    Ok(())
}

fn require_chord_reaches_qol(
    guest: &mut GuestControlClient,
    qmp: &mut qmp::QmpClient,
) -> Result<()> {
    let cursor = current_trace_cursor(guest)?;
    qmp.send_keys(&["shift".into(), "meta_l".into(), "s".into()])?;
    thread::sleep(KEY_SETTLE);
    wait_for_probe_line(
        guest,
        cursor,
        "HOTKEY_DISPATCH",
        "uid=e8208e3e-58b3-4f8c-ad4b-ddbecafa3375 action=open",
        ACTION_TIMEOUT,
    )
    .context("the chord was not dispatched by the tray's hotkey listener")?;
    wait_for_command(
        guest,
        command("/usr/bin/xdotool", &["search", "--name", "^qol-launcher@"]),
        ACTION_TIMEOUT,
        |outcome| !outcome.stdout.trim().is_empty(),
        "the launcher window opened by the reclaimed chord",
    )?;
    qmp.send_keys(&["esc".into()])?;
    thread::sleep(KEY_SETTLE);
    step_label(
        "chord",
        StepKind::Success,
        "Shift+Super+S dispatched to the launcher on the fresh boot",
    );
    Ok(())
}

fn require_de_stays_quiet(guest: &mut GuestControlClient) -> Result<()> {
    let outcome = exec(
        guest,
        command("/usr/bin/test", &["-e", DE_MARKER]),
        COMMAND_TIMEOUT,
    )?;
    if outcome.exit_code == Some(0) {
        bail!("the desktop shortcut ran its command on the fresh boot");
    }
    step_label(
        "de",
        StepKind::Success,
        "the desktop shortcut never fired on the fresh boot",
    );
    Ok(())
}

fn boot_diagnostics(
    guest: &mut GuestControlClient,
    auth: &str,
    error: anyhow::Error,
) -> anyhow::Error {
    let mut detail = Vec::new();
    for (label, spec) in [
        (
            "de-marker",
            command(
                "/usr/bin/bash",
                &["-lc", "ls -la /tmp/de-shadow-evidence 2>&1 || true"],
            ),
        ),
        (
            "binding",
            command(
                "/usr/bin/dconf",
                &["read", "/org/cinnamon/desktop/keybindings/custom-keybindings/custom9/binding"],
            ),
        ),
        (
            "custom-list",
            command(
                "/usr/bin/dconf",
                &["read", "/org/cinnamon/desktop/keybindings/custom-list"],
            ),
        ),
        (
            "claims",
            command(
                "/bin/sh",
                &[
                    "-c",
                    "ls -la /home/qol/.local/share/qol-tray/host-takeover/qol-tray-hotkeys/ 2>&1 || true",
                ],
            ),
        ),
        (
            "trace",
            command(
                "/usr/bin/bash",
                &[
                    "-lc",
                    &format!("grep -E 'HOTKEY_|doctor' {TRACE_LOG_PATH} 2>/dev/null | tail -20 || true"),
                ],
            ),
        ),
        (
            "hotkeys-file",
            command(
                "/usr/bin/bash",
                &[
                    "-lc",
                    "stat -c '%y %s %n' /home/qol/.config/qol-tray/profile/default/os/linux/hotkeys.json 2>&1; cat /home/qol/.config/qol-tray/profile/default/os/linux/hotkeys.json 2>/dev/null || true",
                ],
            ),
        ),
        (
            "watch",
            command(
                "/usr/bin/bash",
                &["-lc", "cat /home/qol/hotkeys-watch.log 2>/dev/null | tail -300 || true"],
            ),
        ),
        (
            "runner-dir",
            command(
                "/usr/bin/bash",
                &["-lc", "ls -la /tmp/qol-guest-runner/ 2>/dev/null; for f in /tmp/qol-guest-runner/*/stderr; do echo \"== $f: $(wc -c < $f 2>/dev/null)\"; tail -5 \"$f\" 2>/dev/null; done"],
            ),
        ),
        (
            "staging-trace",
            command(
                "/usr/bin/bash",
                &[
                    "-lc",
                    &format!("grep -E 'PROFILE_CONFIG_MATERIALIZE|HOTKEY_|SHORTCUTS|SHORT' {TRACE_LOG_PATH} 2>/dev/null | head -40 || true"),
                ],
            ),
        ),
        (
            "sync-state",
            command(
                "/usr/bin/bash",
                &[
                    "-lc",
                    "find /home/qol/.config/qol-tray/profile/default/sync /home/qol/.config/qol-tray/profile/default/device -type f 2>/dev/null | head -12; echo ---; cat /home/qol/.config/qol-tray/profile/default/manifest.json 2>/dev/null; echo; ls -la /home/qol/.config/qol-tray/profile/default/sync/ 2>/dev/null | head -10; ls -la /home/qol/.config/qol-tray/profile/default/os/linux/ 2>/dev/null",
                ],
            ),
        ),
        (
            "profile-tree",
            command(
                "/usr/bin/bash",
                &[
                    "-lc",
                    "find /home/qol/.config/qol-tray/profile -maxdepth 4 2>/dev/null | head -40; echo ---; ls -la /home/qol/.config/qol-tray/profile/default/ 2>/dev/null | head -12",
                ],
            ),
        ),
        (
            "migrations",
            command(
                "/usr/bin/bash",
                &[
                    "-lc",
                    "ls -la /home/qol/.config/qol-tray/migrations/applied/ 2>&1; for f in /home/qol/.config/qol-tray/migrations/applied/*.done; do [ -f \"$f\" ] && echo \"$f: $(cat \"$f\")\"; done",
                ],
            ),
        ),
        (
            "mode",
            command(
                "/usr/bin/bash",
                &[
                    "-lc",
                    "cat /home/qol/.config/qol-tray/mode.json 2>&1; ls -la /home/qol/.config/qol-tray/ 2>&1 | head -12",
                ],
            ),
        ),
        (
            "session-log",
            command(
                "/usr/bin/bash",
                &[
                    "-lc",
                    "grep -iE 'qol|doctor|hotkey|panic|hotkeys' /home/qol/.xsession-errors 2>/dev/null | tail -12 || true",
                ],
            ),
        ),
        (
            "tray-log",
            command(
                "/usr/bin/bash",
                &[
                    "-lc",
                    "cat /home/qol/.local/share/qol-tray/logs/qol-tray.*.log 2>/dev/null | tail -120 || true",
                ],
            ),
        ),
        (
            "trace-full",
            command(
                "/usr/bin/bash",
                &["-lc", &format!("cat {TRACE_LOG_PATH} 2>/dev/null | tail -60 || true")],
            ),
        ),
    ] {
        if let Ok(outcome) = exec(guest, spec, COMMAND_TIMEOUT) {
            detail.push(format!("{label}: {}", outcome.stdout.trim()));
        }
    }
    let hotkeys = command(
        "/usr/bin/curl",
        &[
            "--silent",
            "--header",
            auth,
            &format!("{}/api/hotkeys", local_base_url()),
        ],
    );
    if let Ok(outcome) = exec(guest, hotkeys, COMMAND_TIMEOUT) {
        detail.push(format!("hotkeys-api: {}", outcome.stdout.trim()));
    }
    anyhow::anyhow!("{error}\n--- boot diagnostics ---\n{}", detail.join("\n"))
}

fn claim_markers(guest: &mut GuestControlClient) -> Result<Vec<String>> {
    let outcome = require_exec(
        guest,
        command(
            "/bin/sh",
            &[
                "-c",
                &format!("find /home -type f -path '{CLAIMS_GLOB}' 2>/dev/null || true"),
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

fn write_artifacts(
    vm: &BootedVm,
    guest: &mut GuestControlClient,
    auth: &str,
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
    let dump_path = dir.join("hotkey-shadow-boot-keybindings.ini");
    std::fs::write(&dump_path, &dump.stdout)
        .with_context(|| format!("failed to write {}", dump_path.display()))?;
    let trace = require_exec(
        guest,
        command("/usr/bin/grep", &["HOTKEY_", TRACE_LOG_PATH]),
        COMMAND_TIMEOUT,
    )?;
    let trace_path = dir.join("hotkey-shadow-boot-trace.log");
    std::fs::write(&trace_path, &trace.stdout)
        .with_context(|| format!("failed to write {}", trace_path.display()))?;
    let hotkeys = require_exec(
        guest,
        command(
            "/usr/bin/curl",
            &[
                "--silent",
                "--header",
                auth,
                &format!("{}/api/hotkeys", local_base_url()),
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    let hotkeys_path = dir.join("hotkey-shadow-boot-hotkeys.json");
    std::fs::write(&hotkeys_path, &hotkeys.stdout)
        .with_context(|| format!("failed to write {}", hotkeys_path.display()))?;
    Ok(vec![dump_path, trace_path, hotkeys_path])
}
