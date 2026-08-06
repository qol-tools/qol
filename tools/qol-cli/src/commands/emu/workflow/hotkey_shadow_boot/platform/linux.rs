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
const CHORD_RETRY_TIMEOUT: Duration = Duration::from_secs(10);
const KEY_SETTLE: Duration = Duration::from_millis(200);
const POKE_SETTLE: Duration = Duration::from_secs(5);
const ACTIVE_GRAB_WAIT: Duration = Duration::from_secs(120);
const ACTIVE_GRAB_POLL: Duration = Duration::from_secs(3);
const LAUNCHER_UID: &str = "5cc75f62-2e3b-463c-ac7b-ae269cff1ef1";
const QOL_COMBO: &str = "Shift+Super+S";
const GTK_COMBO: &str = "['<Shift><Super>s']";
const DISPATCH_NEEDLE: &str = "uid=e8208e3e-58b3-4f8c-ad4b-ddbecafa3375 action=open";
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
    let mut auth = String::new();
    for attempt in 0..6 {
        match wait_for_autostart_tray(&mut guest) {
            Ok(authed) => {
                auth = authed;
                break;
            }
            Err(error) if is_connection_error(&error) && attempt < 5 => {
                step_label(
                    "tray",
                    StepKind::Pending,
                    &format!(
                        "guest-control dropped during the boot wait; reconnecting (attempt {})",
                        attempt + 2
                    ),
                );
                drop(guest);
                guest = connect_after_reboot(vm)?;
            }
            Err(error) => return Err(boot_diagnostics(&mut guest, "", error)),
        }
    }

    require_binding(
        &mut guest,
        MANAGED_KEY,
        CLEARED,
        "after the boot doctor pass",
    )
    .map_err(|error| boot_diagnostics(&mut guest, &auth, error))?;
    require_boot_claim(&mut guest)?;
    let mut qmp = qmp::connect_verified(vm.qmp_port, COMMAND_TIMEOUT, &vm.run_id)?;
    require_chord_reaches_qol(&mut guest, &mut qmp).map_err(|error| {
        let error = diagnose_chord_eaten(&mut guest, &mut qmp, error);
        boot_diagnostics(&mut guest, &auth, error)
    })?;
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
    disarm_screensaver(guest)?;
    stage_watcher(guest)?;
    stage_key_eavesdrop(guest)?;
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
    match exec(
        guest,
        command("/usr/bin/systemctl", &["reboot"]),
        COMMAND_TIMEOUT,
    ) {
        Ok(outcome) if outcome.exit_code == Some(0) => Ok(()),
        Ok(outcome) => bail!(
            "systemctl reboot failed: exit={:?}, stderr={}",
            outcome.exit_code,
            outcome.stderr.trim()
        ),
        Err(error) if is_connection_error(&error) => {
            step_label(
                "reboot",
                StepKind::Pending,
                "the guest dropped the control connection during shutdown; reboot accepted",
            );
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn is_connection_error(error: &anyhow::Error) -> bool {
    let text = format!("{error:#}");
    [
        "guest sent a duplicate hello frame",
        "guest-control connection closed",
        "guest-control response timed out",
        "failed to read guest-control frame",
        "failed to write guest-control frame",
        "guest-control connection cancelled",
        "broken pipe",
        "connection reset",
        "connection closed",
        "operation timed out",
    ]
    .iter()
    .any(|marker| text.contains(marker))
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
        DISPATCH_NEEDLE,
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

fn diagnose_chord_eaten(
    guest: &mut GuestControlClient,
    qmp: &mut qmp::QmpClient,
    error: anyhow::Error,
) -> anyhow::Error {
    let mut observations = vec![];
    let active_grab_before = active_grab_probe(guest);
    observations.push(format!("grab-state before: {}", grab_probe(guest)));
    observations.push(format!(
        "active-keyboard-grab before: {active_grab_before} (an active grab silences every passive grab on the display, including the tray's, and leaves the passive probe reading free)"
    ));
    observations.push(format!(
        "de-marker before, so the desktop shortcut ran its own command on the eaten chord: {}",
        marker_exists(guest)
    ));
    let mut dispatched = false;
    for attempt in 0..3 {
        let outcome = if attempt == 0 {
            eavesdrop_chord(guest, qmp)
        } else {
            resend_chord(guest, qmp, CHORD_RETRY_TIMEOUT)
                .map(|dispatched| (dispatched, String::new()))
        };
        match outcome {
            Ok((true, events)) => {
                dispatched = true;
                observations.push(format!(
                    "chord resend {attempt} dispatched: true{} (the first chord likely lost an injection race)",
                    if events.is_empty() {
                        String::new()
                    } else {
                        format!("\ninjected key events: {events}")
                    }
                ));
                break;
            }
            Ok((false, events)) => {
                observations.push(format!("chord resend {attempt} dispatched: false"));
                if !events.is_empty() {
                    observations.push(format!("injected key events: {events}"));
                }
            }
            Err(resend_error) => {
                return anyhow::anyhow!(
                    "{error}\n--- stale-grab experiment (aborted) ---\n{}\nchord resend aborted: {resend_error:#}",
                    observations.join("\n")
                )
            }
        }
    }
    let mut grab_after_poke = None;
    let mut grab_after_csd_kill = None;
    let mut dispatched_once_grab_cleared = None;
    let mut stand_in_grabber_sees_chord = None;
    if !dispatched {
        observations.push(format!(
            "desktop input state at the eat: {}",
            desktop_input_state(guest)
        ));
        if active_grab_before != "none" {
            dispatched_once_grab_cleared = Some(chord_after_active_grab_clears(
                guest,
                qmp,
                &mut observations,
            ));
        }
    }
    if !dispatched && dispatched_once_grab_cleared != Some(true) {
        observations.push(
            "the tray holds this exact core grab on the root window, so every probe above reads held whoever else grabbed the chord; killing the tray (SIGKILL, so its takeover ledger does not restore the seeded binding) is what makes a later held reading evidence of a second holder, and it runs before any dconf poke so the reading is of the boot state the chord actually met"
                .to_string(),
        );
        let probe_without_tray = kill_and_probe(
            guest,
            &mut observations,
            "the tray",
            "pkill -9 -f '/home/qol/.local/bin/qol-tray'",
        );
        observations.push(format!(
            "active-keyboard-grab with the tray dead: {}",
            active_grab_probe(guest)
        ));
        stand_in_grabber_sees_chord = Some(chord_reaches_a_stand_in_grabber(
            guest,
            qmp,
            &mut observations,
        ));
        name_the_active_grab_owner(guest, &mut observations);
        if probe_without_tray == "held" {
            match poke_binding(guest) {
                Ok(()) => {
                    let probe = grab_probe(guest);
                    observations.push(format!(
                        "grab-state after the write-then-clear poke: {probe} (held here means the desktop keeps the grab even when it sees the value change)"
                    ));
                    grab_after_poke = Some(probe);
                }
                Err(poke_error) => {
                    observations.push(format!("poke write-then-clear failed: {poke_error:#}"))
                }
            }
            if grab_after_poke.as_deref() != Some("free") {
                grab_after_csd_kill = Some(kill_and_probe(
                    guest,
                    &mut observations,
                    "the settings daemon",
                    "pkill -9 -f 'csd-media-keys|csd-keybindings|csd-settings-remap|csd-keyboard'",
                ));
            }
        } else {
            observations.push(
                "no second holder at the moment of the eat, so the poke and the settings-daemon kill are skipped: they would only disturb the session that still has to answer for it"
                    .to_string(),
            );
        }
    }
    observations.push(format!("de-marker after: {}", marker_exists(guest)));
    let marker_at_eat = observations
        .iter()
        .any(|line| line.starts_with("de-marker before") && line.ends_with("true"));
    if dispatched_once_grab_cleared == Some(true) {
        return anyhow::anyhow!(
            "{error}\n--- stale-grab experiment ---\n{}\nVERDICT: the chord was eaten for exactly as long as another client held an active keyboard grab and dispatched on the first resend after that grab cleared, with the tray's registration untouched throughout; an active grab silences every passive grab on the display, so this boot eat is not a desktop shortcut shadow and no takeover change can fix it. The owner of that grab, in the desktop input state above, is what has to be handled.",
            observations.join("\n")
        );
    }
    let verdict = match (
        dispatched,
        marker_at_eat,
        stand_in_grabber_sees_chord,
        grab_after_poke.as_deref(),
        grab_after_csd_kill.as_deref(),
    ) {
        (true, _, _, _, _) => {
            "VERDICT: a resend dispatched after the failure, so the first chord lost an injection race and no stale desktop grab is proven; the shadow theory needs a run where no resend dispatches"
                .to_string()
        }
        (false, true, _, _, _) => {
            "VERDICT: the desktop ran the seeded shortcut's own command on the eaten chord, so it still held both the grab and the command the takeover was supposed to clear; the doctor's write of an empty value never reached the desktop's runtime. Note that the core passive probe cannot see this: the desktop grabs through XI2 and the X server tracks core and XI2 passive grabs separately, so a core probe reads free while the desktop is holding the chord. The fix must make the desktop re-read the binding, by removing the keybinding entry rather than emptying its value, or by re-applying the clear once the desktop has settled."
                .to_string()
        }
        (false, false, Some(false), _, _) => {
            "VERDICT: with the tray dead and the desktop binding cleared, a fresh core grabber of the same chord never received the key either, and the desktop ran no command; something the core grab layer cannot see is intercepting the chord, which is the signature of a retained XI2 grab whose command the clear did empty. The fix is the same as the loud case: make the desktop drop the grab, not just blank the value."
                .to_string()
        }
        (false, false, Some(true), _, _) => {
            "VERDICT: with the tray dead, a fresh core grabber of the same chord did receive the key, so nothing on the display is intercepting it and the desktop ran no command; the eat is inside the tray's own hotkey listener, which held a healthy registration and never dispatched"
                .to_string()
        }
        (false, _, None, poke, after_kill) => format!(
            "VERDICT: the stand-in grabber never ran, so the interception question is unanswered; poke = {poke:?}, settings-daemon kill = {after_kill:?}"
        ),
    };
    anyhow::anyhow!(
        "{error}\n--- stale-grab experiment ---\n{}\n{verdict}",
        observations.join("\n")
    )
}

fn name_the_active_grab_owner(guest: &mut GuestControlClient, observations: &mut Vec<String>) {
    if active_grab_probe(guest) == "none" {
        observations.push(
            "the active keyboard grab was already gone before the owner ladder ran".to_string(),
        );
        return;
    }
    for (subject, kill_command) in [
        ("qol's own plugin processes", "pkill -9 -f 'plugin-'"),
        ("the cinnamon shell", "pkill -9 -f 'cinnamon --replace'"),
    ] {
        let killed = exec(
            guest,
            command(
                "/usr/bin/bash",
                &["-lc", &format!("{kill_command}; echo killed")],
            ),
            COMMAND_TIMEOUT,
        );
        if let Err(kill_error) = killed {
            observations.push(format!("{subject} kill aborted: {kill_error:#}"));
            continue;
        }
        thread::sleep(Duration::from_millis(800));
        let state = active_grab_probe(guest);
        observations.push(format!(
            "active-keyboard-grab after killing {subject}: {state}"
        ));
        if state == "none" {
            observations.push(format!(
                "the active keyboard grab died with {subject}, which names its owner"
            ));
            return;
        }
    }
}

fn chord_reaches_a_stand_in_grabber(
    guest: &mut GuestControlClient,
    qmp: &mut qmp::QmpClient,
    observations: &mut Vec<String>,
) -> bool {
    let script = r#"import os, time
from Xlib import X, XK, display

log = open("/home/qol/stand-in-grabber.log", "w")


def say(line):
    log.write(line + "\n")
    log.flush()


d = display.Display(os.environ.get("DISPLAY", ":0"))
root = d.screen().root
keycode = d.keysym_to_keycode(XK.string_to_keysym("s"))
mods = X.ShiftMask | X.Mod4Mask
for extra in (0, X.Mod2Mask, X.LockMask, X.Mod2Mask | X.LockMask):
    root.grab_key(keycode, mods | extra, False, X.GrabModeAsync, X.GrabModeAsync)
d.sync()
say("grabbed keycode=%d" % keycode)
deadline = time.time() + 14
while time.time() < deadline:
    while d.pending_events():
        e = d.next_event()
        if e.type == X.KeyPress:
            say("received keycode=%d state=0x%x" % (e.detail, e.state))
    time.sleep(0.02)
say("window-closed")
d.close()
"#;
    let pid = match write_and_spawn_python(guest, "stand-in-grabber.py", script) {
        Ok(pid) => pid,
        Err(spawn_error) => {
            observations.push(format!("stand-in grabber failed to start: {spawn_error:#}"));
            return false;
        }
    };
    thread::sleep(Duration::from_millis(1200));
    if let Err(key_error) = qmp.send_keys(&["shift".into(), "meta_l".into(), "s".into()]) {
        observations.push(format!("stand-in grabber chord aborted: {key_error:#}"));
        return false;
    }
    thread::sleep(Duration::from_secs(2));
    let outcome = exec(
        guest,
        command(
            "/usr/bin/bash",
            &[
                "-lc",
                &format!(
                    "cat /home/qol/stand-in-grabber.log 2>/dev/null; echo ---runner---; cat /tmp/qol-guest-runner/{pid}/stderr 2>/dev/null | tail -4"
                ),
            ],
        ),
        COMMAND_TIMEOUT,
    );
    let report = match outcome {
        Ok(outcome) => outcome.stdout.trim().replace('\n', " | "),
        Err(read_error) => format!("stand-in grabber log unreadable: {read_error:#}"),
    };
    let received = report.contains("received keycode=");
    observations.push(format!(
        "a fresh core grabber, with the tray dead, {} the chord: {report}",
        if received { "received" } else { "never saw" }
    ));
    observations.push(format!(
        "de-marker right after that injection: {}",
        marker_exists(guest)
    ));
    received
}

fn write_and_spawn_python(
    guest: &mut GuestControlClient,
    file_name: &str,
    script: &str,
) -> Result<u64> {
    require_exec(
        guest,
        command(
            "/usr/bin/sh",
            &[
                "-c",
                &format!("cat > /home/qol/{file_name} <<'PY'\n{script}PY"),
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    spawn(
        guest,
        command("/usr/bin/python3", &[&format!("/home/qol/{file_name}")]),
    )
}

fn desktop_input_state(guest: &mut GuestControlClient) -> String {
    let outcome = exec(
        guest,
        command(
            "/usr/bin/bash",
            &[
                "-lc",
                "echo focus=$(xdotool getwindowfocus getwindowname 2>&1); echo windows=$(wmctrl -l 2>&1 | tr '\\n' '/'); echo screensaver=$(cinnamon-screensaver-command -q 2>&1); echo grabbers=$(ps -eo pid,args | grep -E '[c]innamon|[p]lugin-|[q]ol-' | grep -v qol-guest-runner | tr '\\n' '/')",
            ],
        ),
        COMMAND_TIMEOUT,
    );
    match outcome {
        Ok(outcome) => outcome.stdout.trim().replace('\n', " | "),
        Err(state_error) => format!("state-error: {state_error:#}"),
    }
}

fn chord_after_active_grab_clears(
    guest: &mut GuestControlClient,
    qmp: &mut qmp::QmpClient,
    observations: &mut Vec<String>,
) -> bool {
    let deadline = std::time::Instant::now() + ACTIVE_GRAB_WAIT;
    let mut samples = Vec::new();
    loop {
        let state = active_grab_probe(guest);
        samples.push(state.clone());
        if state == "none" {
            break;
        }
        if std::time::Instant::now() >= deadline {
            observations.push(format!(
                "the active keyboard grab never cleared within {}s: {}",
                ACTIVE_GRAB_WAIT.as_secs(),
                samples.join(" -> ")
            ));
            return false;
        }
        thread::sleep(ACTIVE_GRAB_POLL);
    }
    observations.push(format!(
        "the active keyboard grab cleared after {} samples: {}",
        samples.len(),
        samples.join(" -> ")
    ));
    observations.push(format!(
        "desktop input state once the grab cleared: {}",
        desktop_input_state(guest)
    ));
    match resend_chord(guest, qmp, CHORD_RETRY_TIMEOUT) {
        Ok(dispatched) => {
            observations.push(format!(
                "chord once the active grab cleared dispatched: {dispatched}"
            ));
            dispatched
        }
        Err(resend_error) => {
            observations.push(format!(
                "chord once the active grab cleared aborted: {resend_error:#}"
            ));
            false
        }
    }
}

fn kill_and_probe(
    guest: &mut GuestControlClient,
    observations: &mut Vec<String>,
    subject: &str,
    kill_command: &str,
) -> String {
    let listing = exec(
        guest,
        command(
            "/usr/bin/bash",
            &["-lc", &format!("{kill_command}; echo killed")],
        ),
        COMMAND_TIMEOUT,
    );
    if let Err(kill_error) = listing {
        observations.push(format!("{subject} kill aborted: {kill_error:#}"));
    }
    thread::sleep(Duration::from_millis(600));
    let probe = grab_probe(guest);
    observations.push(format!("grab-state with {subject} dead: {probe}"));
    probe
}

fn poke_binding(guest: &mut GuestControlClient) -> Result<()> {
    dconf_write(guest, MANAGED_KEY, GTK_COMBO)?;
    thread::sleep(POKE_SETTLE);
    dconf_write(guest, MANAGED_KEY, CLEARED)?;
    thread::sleep(POKE_SETTLE);
    Ok(())
}

fn resend_chord(
    guest: &mut GuestControlClient,
    qmp: &mut qmp::QmpClient,
    timeout: Duration,
) -> Result<bool> {
    let cursor = current_trace_cursor(guest)?;
    qmp.send_keys(&["shift".into(), "meta_l".into(), "s".into()])?;
    thread::sleep(KEY_SETTLE);
    Ok(wait_for_probe_line(guest, cursor, "HOTKEY_DISPATCH", DISPATCH_NEEDLE, timeout).is_ok())
}

fn eavesdrop_chord(
    guest: &mut GuestControlClient,
    qmp: &mut qmp::QmpClient,
) -> Result<(bool, String)> {
    let pid = spawn(
        guest,
        command("/usr/bin/python3", &["/home/qol/key-eavesdrop.py"]),
    )?;
    thread::sleep(Duration::from_millis(700));
    let cursor = current_trace_cursor(guest)?;
    qmp.send_keys(&["shift".into(), "meta_l".into(), "s".into()])?;
    thread::sleep(KEY_SETTLE);
    let dispatched = wait_for_probe_line(
        guest,
        cursor,
        "HOTKEY_DISPATCH",
        DISPATCH_NEEDLE,
        CHORD_RETRY_TIMEOUT,
    )
    .is_ok();
    thread::sleep(Duration::from_millis(700));
    let outcome = exec(
        guest,
        command(
            "/usr/bin/bash",
            &[
                "-lc",
                &format!(
                    "cat /home/qol/key-eavesdrop.log 2>/dev/null | head -30 || true; echo ---runner---; cat /tmp/qol-guest-runner/{pid}/stderr 2>/dev/null | tail -6"
                ),
            ],
        ),
        COMMAND_TIMEOUT,
    );
    let events = outcome
        .map(|outcome| outcome.stdout.trim().to_string())
        .unwrap_or_else(|_| "probe-error".to_string());
    Ok((dispatched, events))
}

fn active_grab_probe(guest: &mut GuestControlClient) -> String {
    let script = r#"import os
from Xlib import X, display

d = display.Display(os.environ.get("DISPLAY", ":0"))
root = d.screen().root
status = root.grab_keyboard(False, X.GrabModeAsync, X.GrabModeAsync, X.CurrentTime)
code = getattr(status, "status", status)
names = {0: "none", 1: "held-by-another-client", 2: "invalid-time", 3: "not-viewable", 4: "frozen"}
if code == 0:
    d.ungrab_keyboard(X.CurrentTime)
    d.sync()
print(names.get(code, "status-%s" % code))
d.close()
"#;
    run_python_probe(guest, "active-grab-probe.py", script)
}

fn grab_probe(guest: &mut GuestControlClient) -> String {
    let script = r#"import os
from Xlib import X, XK, display
from Xlib.error import BadAccess

result = ["free"]

def handler(error, request):
    if isinstance(error, BadAccess):
        result[0] = "held"
    else:
        result[0] = "probe-error: %s" % error

d = display.Display(os.environ.get("DISPLAY", ":0"))
d.set_error_handler(handler)
root = d.screen().root
keycode = d.keysym_to_keycode(XK.string_to_keysym("s"))
mods = X.ShiftMask | X.Mod4Mask
try:
    root.grab_key(keycode, mods, False, X.GrabModeAsync, X.GrabModeAsync)
    d.sync()
    root.ungrab_key(keycode, mods)
    d.sync()
except Exception as exc:
    result[0] = "probe-error: %s: %s" % (type(exc).__name__, exc)
d.close()
print(result[0])
"#;
    run_python_probe(guest, "grab-probe.py", script)
}

fn run_python_probe(guest: &mut GuestControlClient, file_name: &str, script: &str) -> String {
    let outcome = exec(
        guest,
        command(
            "/usr/bin/bash",
            &[
                "-c",
                &format!(
                    "cat > /home/qol/{file_name} <<'PY'\n{script}PY\n/usr/bin/python3 /home/qol/{file_name} 2>&1"
                ),
            ],
        ),
        COMMAND_TIMEOUT,
    );
    match outcome {
        Ok(outcome) => outcome.stdout.trim().replace('\n', " | "),
        Err(probe_error) => format!("probe-error: {probe_error:#}"),
    }
}

fn marker_exists(guest: &mut GuestControlClient) -> bool {
    exec(
        guest,
        command("/usr/bin/test", &["-e", DE_MARKER]),
        COMMAND_TIMEOUT,
    )
    .map(|outcome| outcome.exit_code == Some(0))
    .unwrap_or(false)
}

fn stage_key_eavesdrop(guest: &mut GuestControlClient) -> Result<()> {
    let script = r#"import os, struct, time
from Xlib import X, XK, display
from Xlib.ext import xinput

log = open("/home/qol/key-eavesdrop.log", "w")


def say(line):
    log.write(line + "\n")
    log.flush()


d = display.Display(os.environ.get("DISPLAY", ":0"))
root = d.screen().root
# A core selection on the root window never sees a key that another client
# grabbed, so it cannot tell a stale grab apart from a chord that never
# reached the server. XI2 raw events are reported before grab arbitration,
# so they answer that question; keep both and label which saw what.
root.change_attributes(event_mask=X.KeyPressMask | X.KeyReleaseMask)
raw_press = getattr(xinput, "RawKeyPress", 13)
raw_release = getattr(xinput, "RawKeyRelease", 14)
raw = "on"
try:
    d.xinput_query_version()
    root.xinput_select_events([
        (xinput.AllMasterDevices, (1 << raw_press) | (1 << raw_release)),
    ])
except Exception as exc:
    raw = "unavailable: %s: %s" % (type(exc).__name__, exc)
say("raw-xi2=%s" % raw)
d.sync()


def name_of(keycode):
    return XK.keysym_to_string(d.keycode_to_keysym(keycode, 0)) or "?"


def detail_of(data):
    # python-xlib leaves the raw-event body unparsed on this server, so read
    # deviceid/time/detail straight out of the xXIRawEvent bytes.
    if not isinstance(data, (bytes, bytearray)) or len(data) < 10:
        return None
    return struct.unpack_from("<I", data, 6)[0]


deadline = time.time() + 12
while time.time() < deadline:
    while d.pending_events():
        e = d.next_event()
        if getattr(e, "evtype", None) in (raw_press, raw_release):
            keycode = getattr(getattr(e, "data", None), "detail", None)
            if keycode is None:
                keycode = detail_of(getattr(e, "data", None))
            if keycode is None:
                say("raw-event unparsed: %r" % (e,))
            else:
                say("raw-%s keycode=%d keysym=%s" % (
                    "press" if e.evtype == raw_press else "release",
                    keycode,
                    name_of(keycode)))
        elif e.type in (X.KeyPress, X.KeyRelease):
            say("core-%s keycode=%d keysym=%s state=0x%x" % (
                "press" if e.type == X.KeyPress else "release",
                e.detail,
                name_of(e.detail),
                e.state))
    time.sleep(0.02)
say("window-closed")
d.close()
"#;
    require_exec(
        guest,
        command(
            "/usr/bin/sh",
            &[
                "-c",
                &format!("cat > /home/qol/key-eavesdrop.py <<'PY'\n{script}PY"),
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
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
                &["-lc", "stat -c '%y %s %n' /tmp/de-shadow-evidence 2>&1 || true"],
            ),
        ),
        (
            "xmodmap",
            command(
                "/usr/bin/bash",
                &[
                    "-lc",
                    "xmodmap -pm 2>&1 | head -14; echo ---; xmodmap -pke 2>&1 | grep -iE 'super|shift|s ' | head -12",
                ],
            ),
        ),
        (
            "csd",
            command(
                "/usr/bin/bash",
                &[
                    "-lc",
                    "ps -eo pid,lstart,args | grep -iE 'csd|settings-daemon' | grep -v grep | head -15; echo ---; pgrep -a -f 'csd|cinnamon' | head -10",
                ],
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
                    "ls -la /home/qol/.local/share/qol-tray/host-takeover/qol-tray-hotkeys/ 2>&1 || true; stat -c '%y %n' /home/qol/.local/share/qol-tray/host-takeover/qol-tray-hotkeys/takeover-* 2>/dev/null || true",
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

fn disarm_screensaver(guest: &mut GuestControlClient) -> Result<()> {
    for (key, value) in [
        ("/org/cinnamon/desktop/session/idle-delay", "uint32 0"),
        ("/org/cinnamon/desktop/screensaver/lock-enabled", "false"),
        (
            "/org/cinnamon/desktop/screensaver/idle-activation-enabled",
            "false",
        ),
    ] {
        dconf_write(guest, key, value)?;
    }
    let hidden = "[Desktop Entry]\nType=Application\nName=cinnamon-screensaver\nHidden=true\n";
    require_exec(
        guest,
        command(
            "/usr/bin/sh",
            &[
                "-c",
                &format!(
                    "install -d -m 0755 /home/qol/.config/autostart && printf '%s' '{hidden}' > /home/qol/.config/autostart/cinnamon-screensaver.desktop"
                ),
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    step_label(
        "screensaver",
        StepKind::Success,
        "screensaver idle activation, locking and autostart disabled so it cannot grab the keyboard on the idle guest",
    );
    Ok(())
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
