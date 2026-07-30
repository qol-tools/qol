//! Linux Mint implementation of the Bluetooth adversarial workflow.

use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use qol_conventions::{local_base_url, TRACE_LOG_PATH};
use qol_dev_guest::{GuestControlClient, ProcessOutcome};

use crate::commands::emu::{qmp, BootedVm};
use crate::progress::{step_label, StepKind};

use super::desktop::{
    command, connect_desktop_guest, fd_count, install_payload, plugin_daemon_pid, require_exec,
    spawn, start_tray_and_wait_plugin, terminate, wait_for_command,
};
use super::Verdict;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const ACTION_TIMEOUT: Duration = Duration::from_secs(20);
const DEVICE_ACTION_SETTLE_TIMEOUT: Duration = Duration::from_secs(60);
const PLUGIN_ID: &str = "plugin-bluetooth";
const CYCLE_COUNT: usize = 20;
const QUERY_FLOOD_COUNT: usize = 40;
const RACE_CYCLE_COUNT: usize = 10;
// btvirt exchanges emulated radio packets over IPv4 broadcast. The Mint VM is
// intentionally offline, so run it in a guest-local namespace with only a
// loopback broadcast route instead of giving the guest external networking.
const MINT_VIRTUAL_RADIO_BASH: &str = concat!(
    "/usr/sbin/ip link set lo up && ",
    "/usr/sbin/ip route add broadcast 255.255.255.255 dev lo && ",
    "exec /usr/bin/btvirt -d -U3"
);

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
    let scanner = default_adapter_address(&mut guest)?;
    let peers = configure_peer_advertisements(&mut guest, &scanner)?;
    test_query_flood(&mut guest, &auth)?;
    test_discovery(&mut guest, &auth)?;
    test_peer_connect_action(&mut guest, &auth, &peers[0], &daemon)?;
    test_power_search_races(&mut guest, &auth, &daemon)?;
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
                "BLUETOOTH_(ACTION|ADAPTER|ADAPTER_POWER|DEVICE|RELOAD|SEARCH|SNAPSHOT|START)|SURFACE_ACTIVATION",
                TRACE_LOG_PATH,
            ],
        ),
        COMMAND_TIMEOUT,
    )?;
    step_label(
        "storm",
        StepKind::Success,
        &format!(
            "no-adapter recovery, 3 virtual controllers, {} peers, {CYCLE_COUNT} discovery cycles, bounded peer connection, duplicate/racing actions, concurrent queries, hotplug, settings, guards, and crash recovery passed",
            peers.len()
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
        (
            "/usr/bin/test",
            vec!["-x", "/usr/bin/unshare"],
            "the unshare namespace tool",
        ),
    ] {
        let outcome = super::desktop::exec(guest, command(program, &args), Duration::from_secs(2))?;
        if outcome.exit_code != Some(0) {
            bail!(
                "Mint image is missing {description}; rebuild the declared image revision before running bluetooth-storm"
            );
        }
    }
    let namespace = super::desktop::exec(
        guest,
        command(
            "/usr/bin/unshare",
            &[
                "--user",
                "--map-root-user",
                "--net",
                "/usr/bin/bash",
                "-lc",
                concat!(
                    "/usr/sbin/ip link set lo up && ",
                    "/usr/sbin/ip route add broadcast 255.255.255.255 dev lo && ",
                    "/usr/sbin/ip route get 255.255.255.255"
                ),
            ],
        ),
        Duration::from_secs(2),
    )?;
    if namespace.exit_code != Some(0) {
        bail!(
            "Mint image cannot create the isolated broadcast namespace required by the BlueZ emulator"
        );
    }
    Ok(())
}

fn start_virtual_radios(guest: &mut GuestControlClient) -> Result<u64> {
    spawn(
        guest,
        command(
            "/usr/bin/unshare",
            &[
                "--user",
                "--map-root-user",
                "--net",
                "/usr/bin/bash",
                "-lc",
                MINT_VIRTUAL_RADIO_BASH,
            ],
        ),
    )
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

fn default_adapter_address(guest: &mut GuestControlClient) -> Result<String> {
    let outcome = wait_for_command(
        guest,
        command(
            "/usr/bin/busctl",
            &[
                "get-property",
                "org.bluez",
                "/org/bluez/hci0",
                "org.bluez.Adapter1",
                "Address",
            ],
        ),
        ACTION_TIMEOUT,
        |outcome| busctl_string(&outcome.stdout).is_some_and(is_controller_address),
        "BlueZ to publish the hci0 adapter selected by the plugin",
    )?;
    busctl_string(&outcome.stdout)
        .map(str::to_string)
        .context("BlueZ returned a malformed hci0 adapter address")
}

fn configure_peer_advertisements(
    guest: &mut GuestControlClient,
    scanner: &str,
) -> Result<Vec<String>> {
    let controllers = wait_for_command(
        guest,
        command("/usr/bin/bluetoothctl", &["--timeout", "1", "list"]),
        ACTION_TIMEOUT,
        |outcome| controller_addresses(&outcome.stdout).len() == 3,
        "BlueZ to publish all virtual controllers",
    )?;
    let addresses = peer_controller_addresses(&controllers.stdout, scanner);
    if addresses.len() != 2 {
        bail!(
            "BlueZ published {} peer controllers after excluding scanner {scanner}, expected 2",
            addresses.len()
        );
    }
    for (index, address) in addresses.iter().enumerate() {
        let log_path = format!("/tmp/qol-bt-peer-{}.log", index + 1);
        let script = format!(
            "{{ printf 'select {address}\\nsystem-alias qol-bt-peer-{}\\npower on\\npairable on\\ndiscoverable on\\nadvertise on\\n'; sleep 300; }} | /usr/bin/stdbuf --output=L --error=L /usr/bin/bluetoothctl --timeout 290 >{log_path} 2>&1",
            index + 1,
        );
        spawn(guest, command("/usr/bin/bash", &["-lc", &script]))?;
        wait_for_command(
            guest,
            command("/usr/bin/cat", &[&log_path]),
            ACTION_TIMEOUT,
            |outcome| outcome.stdout.contains("Advertising object registered"),
            &format!("peer {} advertisement registration", index + 1),
        )?;
    }
    Ok(addresses.into_iter().map(str::to_string).collect())
}

fn controller_addresses(output: &str) -> Vec<&str> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_ascii_whitespace();
            (fields.next() == Some("Controller"))
                .then(|| fields.next())
                .flatten()
        })
        .filter(|address| is_controller_address(address))
        .collect()
}

fn peer_controller_addresses<'a>(output: &'a str, scanner: &str) -> Vec<&'a str> {
    controller_addresses(output)
        .into_iter()
        .filter(|address| !address.eq_ignore_ascii_case(scanner))
        .collect()
}

fn is_controller_address(address: &str) -> bool {
    let octets = address.split(':').collect::<Vec<_>>();
    octets.len() == 6
        && octets
            .iter()
            .all(|octet| octet.len() == 2 && octet.chars().all(|ch| ch.is_ascii_hexdigit()))
}

fn busctl_string(output: &str) -> Option<&str> {
    output
        .trim()
        .strip_prefix("s \"")
        .and_then(|value| value.strip_suffix('"'))
}

fn test_power_transitions(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    dispatch(guest, auth, "disable_adapter", "{}")?;
    wait_for_power_state(guest, auth, false)?;
    dispatch(guest, auth, "enable_adapter", "{}")?;
    wait_for_power_state(guest, auth, true)?;
    Ok(())
}

fn test_query_flood(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    let base_url = format!("{}/api/plugins/{PLUGIN_ID}/queries", local_base_url());
    let script = format!(
        "for query in adapter_status search_status devices; do \
         /usr/bin/seq 1 {QUERY_FLOOD_COUNT} | \
         /usr/bin/xargs --max-procs=12 --replace={{}} \
         /usr/bin/curl --fail --silent --output /dev/null \
         --header \"$1\" \"$2/$query\" || exit 1; \
         done"
    );
    require_exec(
        guest,
        command(
            "/usr/bin/bash",
            &["-lc", &script, "qol-bluetooth-query-flood", auth, &base_url],
        ),
        ACTION_TIMEOUT,
    )?;
    step_label(
        "queries",
        StepKind::Success,
        &format!(
            "{} concurrent authenticated query requests passed",
            QUERY_FLOOD_COUNT * 3
        ),
    );
    Ok(())
}

fn test_discovery(guest: &mut GuestControlClient, auth: &str) -> Result<()> {
    dispatch(guest, auth, "start_search", "{}")?;
    wait_for_search_state(guest, auth, true)?;
    let inventory = wait_for_discovered_devices(guest, auth)?;
    let initial_count = inventory["count"].as_u64().unwrap_or_default();
    dispatch(guest, auth, "start_search", "{}")?;
    wait_for_query(guest, auth, "devices", ACTION_TIMEOUT, |payload| {
        payload
            .get("count")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|count| count >= initial_count)
    })?;
    dispatch(guest, auth, "stop_search", "{}")?;
    wait_for_search_state(guest, auth, false)?;

    dispatch(guest, auth, "disable_adapter", "{}")?;
    wait_for_power_state(guest, auth, false)?;
    wait_for_query(guest, auth, "devices", ACTION_TIMEOUT, |payload| {
        payload.get("count").and_then(serde_json::Value::as_u64) == Some(0)
    })?;
    dispatch(guest, auth, "enable_adapter", "{}")?;
    wait_for_power_state(guest, auth, true)?;

    for _ in 2..CYCLE_COUNT {
        dispatch(guest, auth, "start_search", "{}")?;
        wait_for_search_state(guest, auth, true)?;
        dispatch(guest, auth, "stop_search", "{}")?;
        wait_for_search_state(guest, auth, false)?;
    }
    dispatch(guest, auth, "start_search", "{}")?;
    wait_for_search_state(guest, auth, true)?;
    let final_inventory = wait_for_discovered_devices(guest, auth)?;
    dispatch(guest, auth, "stop_search", "{}")?;
    wait_for_search_state(guest, auth, false)?;

    let count = final_inventory["count"].as_u64().unwrap_or_default();
    step_label(
        "radio",
        StepKind::Success,
        &format!(
            "real BlueZ discovery observed {count} peer(s); duplicate search and power-off reset passed"
        ),
    );
    Ok(())
}

fn test_peer_connect_action(
    guest: &mut GuestControlClient,
    auth: &str,
    address: &str,
    daemon: &str,
) -> Result<()> {
    let input = serde_json::json!({ "address": address }).to_string();
    dispatch(guest, auth, "connect_device", &input)?;
    let inventory = wait_for_query(
        guest,
        auth,
        "devices",
        DEVICE_ACTION_SETTLE_TIMEOUT,
        |payload| {
            payload["items"]
                .as_array()
                .and_then(|items| items.iter().find(|item| item["address"] == address))
                .is_some_and(|item| {
                    item["connected"].as_bool() == Some(true)
                        || item["status"]
                            .as_str()
                            .is_some_and(|status| status.starts_with("Connect failed:"))
                })
        },
    )?;
    if daemon_pid(guest)? != daemon {
        bail!("Bluetooth daemon restarted during a virtual peer connection action");
    }
    let connected = inventory["items"]
        .as_array()
        .and_then(|items| items.iter().find(|item| item["address"] == address))
        .and_then(|item| item["connected"].as_bool())
        .unwrap_or(false);
    step_label(
        "connect",
        StepKind::Success,
        if connected {
            "virtual peer connected without wedging the daemon"
        } else {
            "virtual peer failure terminated without wedging the daemon"
        },
    );
    Ok(())
}

fn wait_for_discovered_devices(
    guest: &mut GuestControlClient,
    auth: &str,
) -> Result<serde_json::Value> {
    match wait_for_query(guest, auth, "devices", ACTION_TIMEOUT, |payload| {
        payload
            .get("count")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|count| count > 0)
    }) {
        Ok(inventory) => Ok(inventory),
        Err(error) => {
            let script = format!(
                "/usr/bin/bluetoothctl --timeout 1 devices; \
                 /usr/bin/busctl tree org.bluez; \
                 /usr/bin/grep -E 'BLUETOOTH_(ADAPTER|DEVICE|SEARCH|START)' {TRACE_LOG_PATH} | \
                 /usr/bin/tail -n 120"
            );
            let diagnostics = require_exec(
                guest,
                command("/usr/bin/bash", &["-lc", &script]),
                COMMAND_TIMEOUT,
            )?;
            bail!(
                "{error:#}; BlueZ and plugin diagnostics:\n{}",
                diagnostics.stdout.trim()
            )
        }
    }
}

fn test_power_search_races(guest: &mut GuestControlClient, auth: &str, daemon: &str) -> Result<()> {
    for _ in 0..RACE_CYCLE_COUNT {
        dispatch(guest, auth, "start_search", "{}")?;
        dispatch(guest, auth, "disable_adapter", "{}")?;
        dispatch(guest, auth, "stop_search", "{}")?;
        dispatch(guest, auth, "enable_adapter", "{}")?;
    }
    dispatch(guest, auth, "disable_adapter", "{}")?;
    wait_for_power_state(guest, auth, false)?;
    dispatch(guest, auth, "stop_search", "{}")?;
    dispatch(guest, auth, "enable_adapter", "{}")?;
    wait_for_power_state(guest, auth, true)?;
    wait_for_search_state(guest, auth, false)?;
    if daemon_pid(guest)? != daemon {
        bail!("Bluetooth daemon restarted during power/search race storm");
    }
    step_label(
        "races",
        StepKind::Success,
        &format!(
            "{RACE_CYCLE_COUNT} queued power/search races converged with stable daemon pid={daemon}"
        ),
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
    for query in ["adapter_status", "search_status", "devices"] {
        let evidence = format!(
            "SURFACE_ACTIVATION plugin={PLUGIN_ID} phase=runtime-query query={query} status=200"
        );
        wait_for_command(
            guest,
            command("/usr/bin/grep", &["-F", &evidence, TRACE_LOG_PATH]),
            ACTION_TIMEOUT,
            |outcome| outcome.exit_code == Some(0),
            &format!("native Bluetooth Settings query {query} to authenticate"),
        )?;
    }
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
        command("/usr/bin/pkill", &["--signal", "KILL", "--exact", "btvirt"]),
        COMMAND_TIMEOUT,
    )?;
    terminate(guest, btvirt)?;
    wait_for_adapter_state(guest, auth, false, ACTION_TIMEOUT)?;
    wait_for_query(guest, auth, "devices", ACTION_TIMEOUT, |payload| {
        payload.get("count").and_then(serde_json::Value::as_u64) == Some(0)
    })?;
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
    dispatch(guest, auth, "start_search", "{}")?;
    wait_for_search_state(guest, auth, true)?;
    require_exec(
        guest,
        command("/usr/bin/kill", &["--signal", "KILL", before]),
        COMMAND_TIMEOUT,
    )?;
    let after = wait_for_daemon(guest, Some(before))?;
    wait_for_adapter_state(guest, auth, true, ACTION_TIMEOUT)?;
    wait_for_search_state(guest, auth, false)?;
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
    plugin_daemon_pid(
        guest,
        &["-f", "/plugin-bluetooth/plugin-bluetooth$"],
        "Bluetooth daemon",
    )
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
    use super::{busctl_string, controller_addresses, peer_controller_addresses};

    #[test]
    fn controller_list_parser_ignores_noise_and_default_suffix() {
        let output = "Waiting to connect...\nController 00:11:22:33:44:55 qol [default]\nController AA:BB:CC:DD:EE:FF peer\n";
        assert_eq!(
            controller_addresses(output),
            ["00:11:22:33:44:55", "AA:BB:CC:DD:EE:FF"]
        );
    }

    #[test]
    fn peer_selection_uses_hci0_identity_instead_of_client_default_marker() {
        let output = concat!(
            "Controller 00:11:22:33:44:55 peer-a [default]\n",
            "Controller AA:BB:CC:DD:EE:FF scanner\n",
            "Controller 10:20:30:40:50:60 peer-b\n"
        );
        assert_eq!(
            peer_controller_addresses(output, "AA:BB:CC:DD:EE:FF"),
            ["00:11:22:33:44:55", "10:20:30:40:50:60"]
        );
    }

    #[test]
    fn busctl_string_parser_requires_the_dbus_string_shape() {
        assert_eq!(
            busctl_string("s \"AA:BB:CC:DD:EE:FF\"\n"),
            Some("AA:BB:CC:DD:EE:FF")
        );
        assert_eq!(busctl_string("b true\n"), None);
        assert_eq!(busctl_string("AA:BB:CC:DD:EE:FF\n"), None);
    }
}
