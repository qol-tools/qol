use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use qol_conventions::{local_base_url, TRACE_LOG_PATH};
use qol_dev_guest::{GuestControlClient, ProcessOutcome};

use crate::commands::emu::{qmp, BootedVm};
use crate::progress::{step_label, StepKind};

use super::desktop::{
    command, connect_desktop_guest, install_payload, require_exec, spawn,
    start_tray_and_wait_plugin, wait_for_command,
};
use super::Verdict;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const ACTION_TIMEOUT: Duration = Duration::from_secs(20);
const PLUGIN_ID: &str = "plugin-bluetooth";
const CYCLE_COUNT: usize = 20;

pub(super) fn run(vm: &BootedVm) -> Result<Verdict> {
    let mut guest = connect_desktop_guest(vm)?;
    require_virtual_radio_contract(&mut guest)?;
    install_payload(&mut guest)?;
    let auth = start_tray_and_wait_plugin(&mut guest, PLUGIN_ID)?;
    let daemon = wait_for_daemon(&mut guest, None)?;
    wait_for_adapter_state(&mut guest, &auth, false, ACTION_TIMEOUT)?;
    thread::sleep(Duration::from_secs(8));
    if daemon_pid(&mut guest)? != daemon {
        bail!("Bluetooth daemon restarted while no adapter was present");
    }
    step_label(
        "no-adapter",
        StepKind::Success,
        &format!("bounded unavailable queries and stable daemon pid={daemon}"),
    );

    let btvirt = start_virtual_radios(&mut guest)?;
    wait_for_controller_count(&mut guest, 3)?;
    wait_for_adapter_state(&mut guest, &auth, true, ACTION_TIMEOUT)?;
    test_power_transitions(&mut guest, &auth)?;
    let peers = configure_peer_advertisements(&mut guest)?;
    test_discovery(&mut guest, &auth)?;
    test_action_guards(&mut guest, &auth)?;

    let artifacts_dir = vm.run_dir.join("artifacts");
    std::fs::create_dir_all(&artifacts_dir)
        .with_context(|| format!("failed to create {}", artifacts_dir.display()))?;
    let mut qmp = qmp::connect_verified(vm.qmp_port, COMMAND_TIMEOUT, &vm.run_id)?;
    let settings = artifacts_dir.join("settings.ppm");
    test_settings(&mut guest, &auth, &mut qmp, &settings)?;

    let fds_before = fd_count(&mut guest, &daemon)?;
    test_controller_hotplug(&mut guest, &auth, btvirt, &daemon)?;
    let fds_after = fd_count(&mut guest, &daemon)?;
    if fds_after > fds_before.saturating_add(4) {
        bail!("Bluetooth file descriptors grew from {fds_before} to {fds_after}");
    }
    test_daemon_crash_recovery(&mut guest, &auth, &daemon)?;

    let final_state = artifacts_dir.join("final.ppm");
    qmp.screendump(&final_state)?;
    let traces = require_exec(
        &mut guest,
        command(
            "/usr/bin/grep",
            &[
                "-E",
                "BLUETOOTH_(ACTION|ADAPTER|ADAPTER_POWER|DEVICE|RELOAD|SEARCH|SNAPSHOT|START)",
                TRACE_LOG_PATH,
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    step_label(
        "storm",
        StepKind::Success,
        &format!(
            "no-adapter recovery, 3 virtual controllers, {peers} peers, {CYCLE_COUNT} discovery cycles, hotplug, settings, guards, and crash recovery passed"
        ),
    );
    Ok(Verdict {
        pass: true,
        traces: traces.stdout.lines().map(str::to_string).collect(),
        artifacts: vec![settings, final_state],
    })
}

fn require_virtual_radio_contract(guest: &mut GuestControlClient) -> Result<()> {
    for (program, args, description) in [
        (
            "/usr/bin/test",
            vec!["-x", "/usr/bin/btvirt"],
            "the BlueZ btvirt emulator",
        ),
        (
            "/usr/bin/test",
            vec!["-w", "/dev/vhci"],
            "writable kernel VHCI",
        ),
    ] {
        let outcome = super::desktop::exec(guest, command(program, &args), Duration::from_secs(2))?;
        if outcome.exit_code != Some(0) {
            bail!(
                "Mint image is missing {description}; rebuild the declared image revision before running bluetooth-storm"
            );
        }
    }
    Ok(())
}

fn start_virtual_radios(guest: &mut GuestControlClient) -> Result<u64> {
    spawn(guest, command("/usr/bin/btvirt", &["-d", "-U3"]))
}

fn wait_for_controller_count(guest: &mut GuestControlClient, expected: usize) -> Result<()> {
    wait_for_command(
        guest,
        command(
            "/usr/bin/find",
            &[
                "/sys/class/bluetooth",
                "-maxdepth",
                "1",
                "-name",
                "hci*",
                "-printf",
                ".\n",
            ],
        ),
        ACTION_TIMEOUT,
        |outcome| outcome.stdout.lines().count() == expected,
        &format!("{expected} virtual Bluetooth controllers"),
    )?;
    Ok(())
}

fn configure_peer_advertisements(guest: &mut GuestControlClient) -> Result<usize> {
    let controllers = wait_for_command(
        guest,
        command("/usr/bin/bluetoothctl", &["--timeout", "2", "list"]),
        ACTION_TIMEOUT,
        |outcome| {
            let controllers = controller_addresses(&outcome.stdout);
            controllers.len() == 3
                && controllers
                    .iter()
                    .filter(|(_, is_default)| *is_default)
                    .count()
                    == 1
        },
        "BlueZ to publish all virtual controllers",
    )?;
    let controllers = controller_addresses(&controllers.stdout);
    let addresses = controllers
        .iter()
        .filter_map(|(address, is_default)| (!is_default).then_some(*address))
        .collect::<Vec<_>>();
    for (index, address) in addresses.iter().enumerate() {
        let script = format!(
            "{{ printf 'select {address}\\nsystem-alias qol-bt-peer-{}\\npower on\\npairable on\\ndiscoverable on\\nadvertise on\\n'; sleep 300; }} | /usr/bin/bluetoothctl --timeout 290",
            index + 1
        );
        spawn(guest, command("/usr/bin/bash", &["-lc", &script]))?;
    }
    thread::sleep(Duration::from_secs(2));
    Ok(addresses.len())
}

fn controller_addresses(output: &str) -> Vec<(&str, bool)> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_ascii_whitespace();
            (fields.next() == Some("Controller"))
                .then(|| {
                    fields
                        .next()
                        .map(|address| (address, line.ends_with(" [default]")))
                })
                .flatten()
        })
        .filter(|(address, _)| address.split(':').count() == 6)
        .collect()
}

fn test_power_transitions(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    dispatch(guest, auth, "disable_adapter", "{}")?;
    wait_for_power_state(guest, auth, false)?;
    dispatch(guest, auth, "enable_adapter", "{}")?;
    wait_for_power_state(guest, auth, true)?;
    Ok(())
}

fn test_discovery(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    for _ in 0..CYCLE_COUNT {
        dispatch(guest, auth, "start_search", "{}")?;
        wait_for_search_state(guest, auth, true)?;
        dispatch(guest, auth, "stop_search", "{}")?;
        wait_for_search_state(guest, auth, false)?;
    }
    let inventory = wait_for_query(guest, auth, "devices", ACTION_TIMEOUT, |payload| {
        payload
            .get("count")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|count| count > 0)
    })?;
    let count = inventory["count"].as_u64().unwrap_or_default();
    step_label(
        "radio",
        StepKind::Success,
        &format!("real BlueZ power transitions and discovery observed {count} peer(s)"),
    );
    Ok(())
}

fn test_action_guards(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    for (action, input) in [
        ("pair_device", "{}"),
        ("connect_device", r#"{"address":"not-an-address"}"#),
    ] {
        let status = dispatch_status(guest, Some(auth), action, input)?;
        if status != "409" {
            bail!("{action} invalid-input guard returned {status}, expected 409");
        }
    }
    if dispatch_status(guest, Some(auth), "not-real", "{}")? != "400" {
        bail!("unknown Bluetooth action did not return 400");
    }
    if dispatch_status(guest, None, "start_search", "{}")? != "401" {
        bail!("unauthenticated Bluetooth action did not return 401");
    }
    Ok(())
}

fn test_settings(
    guest: &mut GuestControlClient,
    auth: &str,
    qmp: &mut qmp::QmpClient,
    artifact: &std::path::Path,
) -> Result<()> {
    dispatch(guest, auth, "settings", "{}")?;
    wait_for_command(
        guest,
        command("/usr/bin/xdotool", &["getactivewindow", "getwindowname"]),
        ACTION_TIMEOUT,
        |outcome| outcome.stdout.trim().starts_with("Bluetooth Settings"),
        "native Bluetooth Settings to own guest focus",
    )?;
    qmp.screendump(artifact)?;
    require_exec(
        guest,
        command("/usr/bin/xdotool", &["key", "--clearmodifiers", "Escape"]),
        COMMAND_TIMEOUT,
    )?;
    Ok(())
}

fn test_controller_hotplug(
    guest: &mut GuestControlClient,
    auth: &str,
    btvirt: u64,
    daemon: &str,
) -> Result<()> {
    require_exec(
        guest,
        command("/usr/bin/kill", &["--signal", "KILL", &btvirt.to_string()]),
        COMMAND_TIMEOUT,
    )?;
    wait_for_adapter_state(guest, auth, false, ACTION_TIMEOUT)?;
    if daemon_pid(guest)? != daemon {
        bail!("Bluetooth daemon restarted when the virtual controller disappeared");
    }
    start_virtual_radios(guest)?;
    wait_for_controller_count(guest, 3)?;
    wait_for_adapter_state(guest, auth, true, ACTION_TIMEOUT)?;
    if daemon_pid(guest)? != daemon {
        bail!("Bluetooth daemon restarted when the virtual controller returned");
    }
    step_label(
        "hotplug",
        StepKind::Success,
        &format!("virtual controller removal and recovery retained daemon pid={daemon}"),
    );
    Ok(())
}

fn test_daemon_crash_recovery(
    guest: &mut GuestControlClient,
    auth: &str,
    before: &str,
) -> Result<()> {
    require_exec(
        guest,
        command("/usr/bin/kill", &["--signal", "KILL", before]),
        COMMAND_TIMEOUT,
    )?;
    let after = wait_for_daemon(guest, Some(before))?;
    wait_for_adapter_state(guest, auth, true, ACTION_TIMEOUT)?;
    dispatch(guest, auth, "start_search", "{}")?;
    wait_for_search_state(guest, auth, true)?;
    dispatch(guest, auth, "stop_search", "{}")?;
    step_label(
        "recovery",
        StepKind::Success,
        &format!("tray supervisor recovered Bluetooth pid={before}->{after}"),
    );
    Ok(())
}

fn wait_for_daemon(guest: &mut GuestControlClient, not_pid: Option<&str>) -> Result<String> {
    let outcome = wait_for_command(
        guest,
        command(
            "/usr/bin/pgrep",
            &["-f", "/plugin-bluetooth/plugin-bluetooth$"],
        ),
        ACTION_TIMEOUT,
        |outcome| {
            outcome.stdout.lines().next().is_some_and(|pid| {
                !pid.trim().is_empty() && not_pid.is_none_or(|before| pid.trim() != before)
            })
        },
        "Bluetooth daemon process",
    )?;
    outcome
        .stdout
        .lines()
        .next()
        .map(str::trim)
        .map(str::to_string)
        .context("Bluetooth daemon lookup returned no PID")
}

fn daemon_pid(guest: &mut GuestControlClient) -> Result<String> {
    let outcome = require_exec(
        guest,
        command(
            "/usr/bin/pgrep",
            &["-f", "/plugin-bluetooth/plugin-bluetooth$"],
        ),
        COMMAND_TIMEOUT,
    )?;
    outcome
        .stdout
        .lines()
        .next()
        .map(str::trim)
        .map(str::to_string)
        .context("Bluetooth daemon was not running")
}

fn fd_count(guest: &mut GuestControlClient, pid: &str) -> Result<u64> {
    let path = format!("/proc/{pid}/fd");
    let outcome = require_exec(
        guest,
        command(
            "/usr/bin/find",
            &[&path, "-maxdepth", "1", "-type", "l", "-printf", ".\n"],
        ),
        COMMAND_TIMEOUT,
    )?;
    Ok(outcome.stdout.lines().count() as u64)
}

fn dispatch(guest: &mut GuestControlClient, auth: &str, action: &str, input: &str) -> Result<()> {
    let status = dispatch_status(guest, Some(auth), action, input)?;
    if status != "200" {
        bail!("Bluetooth action {action} returned HTTP {status}");
    }
    Ok(())
}

fn dispatch_status(
    guest: &mut GuestControlClient,
    auth: Option<&str>,
    action: &str,
    input: &str,
) -> Result<String> {
    let url = format!(
        "{}/api/plugins/{PLUGIN_ID}/actions/{action}",
        local_base_url()
    );
    let mut args = vec![
        "--silent",
        "--output",
        "/dev/null",
        "--write-out",
        "%{http_code}",
        "--request",
        "POST",
        "--header",
        "Content-Type: application/json",
        "--data",
        input,
    ];
    if let Some(auth) = auth {
        args.extend(["--header", auth]);
    }
    args.push(&url);
    Ok(
        require_exec(guest, command("/usr/bin/curl", &args), ACTION_TIMEOUT)?
            .stdout
            .trim()
            .to_string(),
    )
}

fn wait_for_adapter_state(
    guest: &mut GuestControlClient,
    auth: &str,
    available: bool,
    timeout: Duration,
) -> Result<serde_json::Value> {
    wait_for_query(guest, auth, "adapter_status", timeout, |payload| {
        payload
            .get("available")
            .and_then(serde_json::Value::as_bool)
            == Some(available)
    })
}

fn wait_for_power_state(
    guest: &mut GuestControlClient,
    auth: &str,
    powered: bool,
) -> Result<serde_json::Value> {
    wait_for_query(guest, auth, "adapter_status", ACTION_TIMEOUT, |payload| {
        payload.get("powered").and_then(serde_json::Value::as_bool) == Some(powered)
    })
}

fn wait_for_search_state(
    guest: &mut GuestControlClient,
    auth: &str,
    searching: bool,
) -> Result<serde_json::Value> {
    wait_for_query(guest, auth, "search_status", ACTION_TIMEOUT, |payload| {
        payload
            .get("searching")
            .and_then(serde_json::Value::as_bool)
            == Some(searching)
    })
}

fn wait_for_query(
    guest: &mut GuestControlClient,
    auth: &str,
    query: &str,
    timeout: Duration,
    predicate: impl Fn(&serde_json::Value) -> bool,
) -> Result<serde_json::Value> {
    let url = format!(
        "{}/api/plugins/{PLUGIN_ID}/queries/{query}",
        local_base_url()
    );
    let outcome = wait_for_command(
        guest,
        command(
            "/usr/bin/curl",
            &["--fail", "--silent", "--header", auth, &url],
        ),
        timeout,
        |outcome| parse_query(outcome).is_some_and(|payload| predicate(&payload)),
        &format!("Bluetooth query {query} to reach the expected state"),
    )?;
    serde_json::from_str(&outcome.stdout)
        .with_context(|| format!("Bluetooth query {query} returned malformed JSON"))
}

fn parse_query(outcome: &ProcessOutcome) -> Option<serde_json::Value> {
    serde_json::from_str(&outcome.stdout).ok()
}

#[cfg(test)]
mod tests {
    use super::controller_addresses;

    #[test]
    fn controller_list_parser_ignores_noise_and_default_suffix() {
        let output = "Waiting to connect...\nController 00:11:22:33:44:55 qol [default]\nController AA:BB:CC:DD:EE:FF peer\n";
        assert_eq!(
            controller_addresses(output),
            [("00:11:22:33:44:55", true), ("AA:BB:CC:DD:EE:FF", false)]
        );
    }
}
