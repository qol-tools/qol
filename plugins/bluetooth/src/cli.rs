use std::process::ExitCode;

use anyhow::{bail, Result};
use qol_headless::{Command, DoctorCheck, DoctorCheckResult, HeadlessApp, PlainTextOutput};
use qol_host_fixes::HostFixes;

use crate::bluetooth::{
    normalize_address, AdapterHealth, DeviceInfo, ReconnectReport, ReconnectSelection,
};
use crate::hostfix::{self, BluetoothHostFixes};
use crate::{config, platform, PLUGIN_ID};

const BINARY_NAME: &str = "plugin-bluetooth";

pub fn exit_code(args: impl IntoIterator<Item = String>) -> ExitCode {
    app().run(args)
}

fn app() -> HeadlessApp {
    HeadlessApp::new(PLUGIN_ID, BINARY_NAME)
        .about("Inspect and reliably reconnect Bluetooth devices through the platform backend.")
        .default_command(["list"])
        .command(list_command())
        .command(enable_adapter_command())
        .command(disable_adapter_command())
        .command(search_command())
        .command(stop_search_command())
        .command(pair_command())
        .command(trust_command())
        .command(untrust_command())
        .command(connect_command())
        .command(disconnect_command())
        .command(remove_command())
        .command(reconnect_command())
        .command(reconnect_trusted_command())
        .command(host_fixes_command())
        .command(apply_host_fix_command())
        .command(settings_command())
        .doctor_checks(doctor_checks())
}

fn enable_adapter_command() -> Command {
    adapter_power_command("enable_adapter", true)
}

fn disable_adapter_command() -> Command {
    adapter_power_command("disable_adapter", false)
}

fn adapter_power_command(name: &'static str, powered: bool) -> Command {
    let intent = if powered {
        "Power on the default Bluetooth adapter."
    } else {
        "Power off the default Bluetooth adapter."
    };
    Command::new(name)
        .about(intent)
        .usage(format!("{BINARY_NAME} {name}"))
        .output("The resulting adapter state.")
        .exit_behavior("Exits non-zero when BlueZ or the default adapter is unavailable.")
        .run_plain_text(move |context| {
            reject_args(context.args())?;
            Ok(PlainTextOutput::text(adapter_health_line(
                &platform::set_adapter_powered(powered)?,
            )))
        })
        .run_json(move |context| {
            reject_args(context.args())?;
            Ok(serde_json::to_value(platform::set_adapter_powered(
                powered,
            )?)?)
        })
}

fn stop_search_command() -> Command {
    Command::new("stop_search")
        .about("Stop discovery in the running Bluetooth daemon.")
        .usage(format!("{BINARY_NAME} stop_search"))
        .output("No stdout on success.")
        .exit_behavior("Exits non-zero when the Bluetooth daemon is not reachable.")
        .run_plain_text(|context| {
            reject_args(context.args())?;
            platform::stop_search()?;
            Ok(PlainTextOutput::empty())
        })
}

fn search_command() -> Command {
    Command::new("search")
        .about("Search for Bluetooth devices for up to 60 seconds.")
        .usage(format!("{BINARY_NAME} search"))
        .detail("Press Ctrl+C to stop discovery early and print the devices found.")
        .output("Paired and discovered devices ordered by connection state and signal strength.")
        .exit_behavior("Exits non-zero when BlueZ discovery cannot run.")
        .run_plain_text(|context| {
            reject_args(context.args())?;
            let devices = platform::search_devices(&config::load())?;
            if devices.is_empty() {
                return Ok(PlainTextOutput::text("no Bluetooth devices found"));
            }
            Ok(PlainTextOutput::text(device_lines(&devices)))
        })
        .run_json(|context| {
            reject_args(context.args())?;
            Ok(serde_json::to_value(platform::search_devices(
                &config::load(),
            )?)?)
        })
}

fn list_command() -> Command {
    Command::new("list")
        .about("List Bluetooth devices known to the default adapter.")
        .usage(format!("{BINARY_NAME} list"))
        .output("One line per known device, or a JSON array with --json.")
        .exit_behavior("Exits non-zero when BlueZ or the default adapter is unavailable.")
        .run_plain_text(|context| {
            reject_args(context.args())?;
            let devices = platform::list_devices()?;
            if devices.is_empty() {
                return Ok(PlainTextOutput::text("no Bluetooth devices known to BlueZ"));
            }
            Ok(PlainTextOutput::text(device_lines(&devices)))
        })
        .run_json(|context| {
            reject_args(context.args())?;
            Ok(serde_json::to_value(platform::list_devices()?)?)
        })
}

fn connect_command() -> Command {
    Command::new("connect")
        .about("Pair, trust, and connect one Bluetooth device in a verified operation.")
        .usage(format!("{BINARY_NAME} connect AA:BB:CC:DD:EE:FF"))
        .detail(
            "Powers on the adapter first when enabled and repairs audio devices that BlueZ only knows through Bluetooth LE.",
        )
        .output("The resulting device state.")
        .exit_behavior("Exits non-zero when the address is invalid or the connection fails.")
        .run_plain_text(|context| {
            let address = one_address("connect", context.args())?;
            let device = platform::connect_device(&address, config::load().power_on_adapter)?;
            Ok(PlainTextOutput::text(device_line(&device)))
        })
        .run_json(|context| {
            let address = one_address("connect", context.args())?;
            let device = platform::connect_device(&address, config::load().power_on_adapter)?;
            Ok(serde_json::to_value(device)?)
        })
}

fn pair_command() -> Command {
    Command::new("pair")
        .about("Pair one Bluetooth device using its appropriate transport.")
        .usage(format!("{BINARY_NAME} pair AA:BB:CC:DD:EE:FF"))
        .output("The resulting device state.")
        .exit_behavior("Exits non-zero when pairing or profile discovery fails.")
        .run_plain_text(|context| {
            let address = one_address("pair", context.args())?;
            let device = platform::pair_device(&address, config::load().power_on_adapter)?;
            Ok(PlainTextOutput::text(device_line(&device)))
        })
        .run_json(|context| {
            let address = one_address("pair", context.args())?;
            let device = platform::pair_device(&address, config::load().power_on_adapter)?;
            Ok(serde_json::to_value(device)?)
        })
}

fn trust_command() -> Command {
    trust_state_command("trust", true)
}

fn untrust_command() -> Command {
    trust_state_command("untrust", false)
}

fn trust_state_command(name: &'static str, trusted: bool) -> Command {
    Command::new(name)
        .about(if trusted {
            "Trust one paired Bluetooth device."
        } else {
            "Remove trust from one paired Bluetooth device."
        })
        .usage(format!("{BINARY_NAME} {name} AA:BB:CC:DD:EE:FF"))
        .output("The resulting device state.")
        .exit_behavior("Exits non-zero when BlueZ cannot update the trust state.")
        .run_plain_text(move |context| {
            let address = one_address(name, context.args())?;
            let device = platform::set_device_trusted(&address, trusted)?;
            Ok(PlainTextOutput::text(device_line(&device)))
        })
        .run_json(move |context| {
            let address = one_address(name, context.args())?;
            let device = platform::set_device_trusted(&address, trusted)?;
            Ok(serde_json::to_value(device)?)
        })
}

fn disconnect_command() -> Command {
    Command::new("disconnect")
        .about("Disconnect every active profile for one Bluetooth device.")
        .usage(format!("{BINARY_NAME} disconnect AA:BB:CC:DD:EE:FF"))
        .output("The resulting device state.")
        .exit_behavior("Exits non-zero when BlueZ cannot disconnect the device.")
        .run_plain_text(|context| {
            let address = one_address("disconnect", context.args())?;
            let device = platform::disconnect_device(&address)?;
            Ok(PlainTextOutput::text(device_line(&device)))
        })
        .run_json(|context| {
            let address = one_address("disconnect", context.args())?;
            let device = platform::disconnect_device(&address)?;
            Ok(serde_json::to_value(device)?)
        })
}

fn remove_command() -> Command {
    Command::new("remove")
        .about("Remove one Bluetooth device and its pairing information.")
        .usage(format!("{BINARY_NAME} remove AA:BB:CC:DD:EE:FF"))
        .output("No stdout on success.")
        .exit_behavior("Exits non-zero when BlueZ cannot remove the device.")
        .run_plain_text(|context| {
            let address = one_address("remove", context.args())?;
            platform::remove_device(&address)?;
            Ok(PlainTextOutput::empty())
        })
}

fn reconnect_command() -> Command {
    Command::new("reconnect")
        .about("Reconnect every device in the managed-device allowlist.")
        .usage(format!("{BINARY_NAME} reconnect"))
        .output("A connection result for each managed device.")
        .exit_behavior("Exits non-zero only when BlueZ itself is unavailable.")
        .run_plain_text(|context| {
            reject_args(context.args())?;
            let report = platform::reconnect_devices(&config::load(), ReconnectSelection::Managed)?;
            Ok(PlainTextOutput::text(report_lines(&report)))
        })
        .run_json(|context| {
            reject_args(context.args())?;
            let report = platform::reconnect_devices(&config::load(), ReconnectSelection::Managed)?;
            Ok(serde_json::to_value(report)?)
        })
}

fn reconnect_trusted_command() -> Command {
    Command::new("reconnect_trusted")
        .about("Reconnect every paired, trusted, disconnected device known to BlueZ.")
        .usage(format!("{BINARY_NAME} reconnect_trusted"))
        .detail("This is an explicit recovery action and does not alter the automatic allowlist.")
        .output("A connection result for each eligible device.")
        .exit_behavior("Exits non-zero only when BlueZ itself is unavailable.")
        .run_plain_text(|context| {
            reject_args(context.args())?;
            let report = platform::reconnect_devices(&config::load(), ReconnectSelection::Trusted)?;
            Ok(PlainTextOutput::text(report_lines(&report)))
        })
        .run_json(|context| {
            reject_args(context.args())?;
            let report = platform::reconnect_devices(&config::load(), ReconnectSelection::Trusted)?;
            Ok(serde_json::to_value(report)?)
        })
}

fn settings_command() -> Command {
    Command::new("settings")
        .about("Open the plugin settings.")
        .usage(format!("{BINARY_NAME} settings"))
        .output("No stdout on success.")
        .exit_behavior("Exits non-zero if native and browser settings cannot be opened.")
        .run_plain_text(|context| {
            reject_args(context.args())?;
            crate::show_settings()?;
            Ok(PlainTextOutput::empty())
        })
}

fn doctor_checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            "bluez_available",
            "Verify the BlueZ system service and default adapter are available.",
            bluez_check,
        ),
        DoctorCheck::new(
            "adapter_powered",
            "Verify the default Bluetooth adapter is powered.",
            adapter_powered_check,
        ),
        DoctorCheck::new(
            "config_readable",
            "Verify the plugin config can be read and parsed without changing it.",
            config_readable_check,
        ),
        DoctorCheck::new(
            "required_binaries",
            "Inspect required platform helper metadata without executing it.",
            || Ok(platform::required_binaries_check()),
        ),
        DoctorCheck::new(
            "managed_devices",
            "Verify the automatic reconnect allowlist contains valid addresses.",
            managed_devices_check,
        ),
        DoctorCheck::new(
            "host_takeover",
            "Verify Bluetooth host ownership left no orphaned Blueman autostart override.",
            host_takeover_check,
        ),
    ]
}

fn bluez_check() -> Result<DoctorCheckResult> {
    let health = platform::adapter_health()?;
    Ok(DoctorCheckResult::ok(
        "bluez_available",
        format!("BlueZ adapter {} is available", health.name),
    )
    .with_details(serde_json::to_value(health)?))
}

fn adapter_powered_check() -> Result<DoctorCheckResult> {
    let health = platform::adapter_health()?;
    if health.powered {
        return Ok(DoctorCheckResult::ok(
            "adapter_powered",
            format!("adapter {} is powered", health.name),
        ));
    }
    Ok(DoctorCheckResult::warn(
        "adapter_powered",
        format!("adapter {} is powered off", health.name),
    )
    .with_fix(format!("run: {BINARY_NAME} reconnect_trusted")))
}

fn config_readable_check() -> Result<DoctorCheckResult> {
    let inspection = config::inspect()?;
    let message = if inspection.source.is_some() {
        "plugin config is readable"
    } else {
        "no plugin config found; contract defaults are valid"
    };
    Ok(DoctorCheckResult::ok("config_readable", message))
}

fn managed_devices_check() -> Result<DoctorCheckResult> {
    let config = config::inspect()?.config;
    let invalid = config
        .managed_devices
        .iter()
        .filter(|address| normalize_address(address).is_err())
        .cloned()
        .collect::<Vec<_>>();
    if !invalid.is_empty() {
        return Ok(DoctorCheckResult::fail(
            "managed_devices",
            format!("invalid managed device addresses: {}", invalid.join(", ")),
        )
        .with_fix("use six colon-separated hex octets per address"));
    }
    if config.managed_devices.is_empty() {
        return Ok(DoctorCheckResult::warn(
            "managed_devices",
            "automatic reconnect has no managed devices",
        )
        .with_fix("add paired device addresses in Bluetooth settings"));
    }
    Ok(DoctorCheckResult::ok(
        "managed_devices",
        format!(
            "{} managed device(s) configured",
            config.managed_devices.len()
        ),
    ))
}

fn host_takeover_check() -> Result<DoctorCheckResult> {
    if hostfix::orphaned_autostart_override() {
        return Ok(DoctorCheckResult::warn(
            "host_takeover",
            "Blueman autostart is hidden by an orphaned qol ownership override",
        )
        .with_fix(format!(
            "run: {BINARY_NAME} apply_host_fix {}",
            hostfix::ORPHANED_AUTOSTART_FIX_ID
        )));
    }
    Ok(DoctorCheckResult::ok(
        "host_takeover",
        "Bluetooth host ownership has no orphaned Blueman autostart override",
    ))
}

fn host_fixes_command() -> Command {
    Command::new("host_fixes")
        .about("Report contextual findings about this computer's Bluetooth stack.")
        .usage(format!("{BINARY_NAME} host_fixes"))
        .output("One line per finding, or the full payload with --json.")
        .exit_behavior("Exits non-zero only when the findings cannot be encoded.")
        .run_plain_text(|context| {
            reject_args(context.args())?;
            Ok(PlainTextOutput::text(finding_lines()))
        })
        .run_json(|context| {
            reject_args(context.args())?;
            Ok(qol_host_fixes::findings_payload(
                &BluetoothHostFixes.detect(),
            ))
        })
}

fn apply_host_fix_command() -> Command {
    Command::new("apply_host_fix")
        .about("Apply one contextual fix to this computer's Bluetooth stack.")
        .usage(format!("{BINARY_NAME} apply_host_fix <fix-id>"))
        .output("The applied fix summary.")
        .exit_behavior("Exits non-zero when the fix id is unknown or the fix fails.")
        .run_plain_text(|context| {
            let id = one_fix_id(context.args())?;
            Ok(PlainTextOutput::text(BluetoothHostFixes.apply(&id)?))
        })
}

fn one_fix_id(args: &[String]) -> Result<String> {
    if args.len() != 1 {
        bail!("apply_host_fix requires exactly one fix id")
    }
    Ok(args[0].clone())
}

fn finding_lines() -> String {
    BluetoothHostFixes
        .detect()
        .iter()
        .map(|finding| format!("{}  {}  {}", finding.id, finding.title, finding.detail))
        .collect::<Vec<_>>()
        .join("\n")
}

fn reject_args(args: &[String]) -> Result<()> {
    if args.is_empty() {
        return Ok(());
    }
    bail!("unexpected arguments: {}", args.join(" "))
}

fn one_address(command: &str, args: &[String]) -> Result<String> {
    if args.len() != 1 {
        bail!("{command} requires exactly one Bluetooth address")
    }
    normalize_address(&args[0])
}

fn device_lines(devices: &[DeviceInfo]) -> String {
    devices
        .iter()
        .map(device_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn device_line(device: &DeviceInfo) -> String {
    format!(
        "{}  {}  paired={} trusted={} connected={} ready={} rssi={}",
        device.address,
        device.alias,
        device.paired,
        device.trusted,
        device.connected,
        crate::bluetooth::connection_ready(device),
        device
            .rssi
            .map(|rssi| rssi.to_string())
            .unwrap_or_else(|| "unavailable".into())
    )
}

fn adapter_health_line(health: &AdapterHealth) -> String {
    format!(
        "{}  {}  powered={}",
        health.name, health.address, health.powered
    )
}

fn report_lines(report: &ReconnectReport) -> String {
    let mut lines = report
        .connected
        .iter()
        .map(|device| format!("connected: {} ({})", device.alias, device.address))
        .collect::<Vec<_>>();
    lines.extend(
        report
            .already_connected
            .iter()
            .map(|device| format!("already connected: {} ({})", device.alias, device.address)),
    );
    lines.extend(report.failures.iter().map(|failure| {
        format!(
            "failed: {} ({}) - {}",
            failure.alias, failure.address, failure.error
        )
    }));
    if lines.is_empty() {
        return "no eligible Bluetooth devices".to_string();
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::app;
    use qol_plugin_api::manifest::PluginManifest;

    #[test]
    fn manifest_actions_have_cli_commands() {
        let manifest =
            PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");

        for action in manifest.executable_actions() {
            let args = manifest
                .catalog_runtime_args(&action.id)
                .expect("executable action must have runtime args");
            let command = args.first().expect("runtime args must name a command");
            let execution = app().execute(vec!["help".to_string(), command.clone()]);

            assert_eq!(
                execution.exit_code,
                qol_headless::EXIT_SUCCESS,
                "action={} command={} stderr={}",
                action.id,
                command,
                execution.stderr
            );
        }
    }

    #[test]
    fn adapter_power_help_is_equivalent_and_structured() {
        for command in ["enable_adapter", "disable_adapter"] {
            let first = app().execute(vec!["help".to_string(), command.to_string()]);
            let final_token = app().execute(vec![command.to_string(), "help".to_string()]);

            assert_eq!(first.exit_code, qol_headless::EXIT_SUCCESS);
            assert_eq!(first.stdout, final_token.stdout, "command={command}");
            assert!(first.stdout.contains("Output:"), "command={command}");
            assert!(
                first.stdout.contains("Supports --json"),
                "command={command}"
            );
            assert!(first.stdout.contains("Exit:"), "command={command}");
        }
    }

    #[test]
    fn doctor_registers_read_only_config_check() {
        let execution = app().execute(vec![
            "doctor".to_string(),
            "config_readable".to_string(),
            "help".to_string(),
        ]);

        assert_eq!(execution.exit_code, qol_headless::EXIT_SUCCESS);
        assert!(execution.stdout.contains("without changing it"));
    }

    #[test]
    fn doctor_registers_required_binary_check() {
        let execution = app().execute(vec![
            "doctor".to_string(),
            "required_binaries".to_string(),
            "help".to_string(),
        ]);

        assert_eq!(execution.exit_code, qol_headless::EXIT_SUCCESS);
        assert!(execution.stdout.contains("without executing it"));
    }
}
