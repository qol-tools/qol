use std::collections::HashMap;
use std::sync::{mpsc, LazyLock, RwLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use qol_host_fixes::{findings_payload, HostFixes};
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_plugin_daemon::notification::send_notification;
use qol_runtime::protocol::{DaemonRequest, DaemonResponse};

use crate::bluetooth::{
    adapter_options, connection_ready, devices_payload, managed_device_options, normalize_address,
    retry::{RetryPolicy, RetryState},
    search_status_payload, AdapterHealth, AdapterInfo, DeviceActionState, DeviceInfo, DeviceIntent,
    DeviceOption, DiscoveryState, ReconnectFailure, ReconnectReport, ReconnectSelection,
};
use crate::config::ReconnectConfig;
use crate::hostfix::BluetoothHostFixes;

pub(super) trait PairedStack {
    const TRUST_REFUSAL: &'static str;
    const RETRY_SPACING: Duration;

    fn paired_devices() -> Vec<DeviceInfo>;
    fn adapter_health() -> Result<AdapterHealth>;
    fn set_adapter_powered(powered: bool) -> Result<AdapterHealth>;
    fn connect_device(address: &str, power_on_adapter: bool) -> Result<DeviceInfo>;
    fn disconnect_device(address: &str) -> Result<DeviceInfo>;
    fn pair_device(address: &str, power_on_adapter: bool) -> Result<DeviceInfo>;
    fn remove_device(address: &str) -> Result<()>;
    fn search_devices(config: &ReconnectConfig) -> Result<Vec<DeviceInfo>>;
    fn pause(slice: Duration);
}

const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

const DAEMON_TICK: Duration = Duration::from_millis(250);

static DISCOVERY_STATE: LazyLock<RwLock<DiscoveryState>> =
    LazyLock::new(|| RwLock::new(DiscoveryState::default()));
static DEVICE_ACTION_STATE: LazyLock<RwLock<Option<DeviceActionState>>> =
    LazyLock::new(|| RwLock::new(None));

pub(super) fn ensure_powered<S: PairedStack>(power_on_adapter: bool) -> Result<()> {
    if S::adapter_health()?.powered {
        return Ok(());
    }
    if !power_on_adapter {
        bail!("the Bluetooth adapter is off");
    }
    S::set_adapter_powered(true)?;
    Ok(())
}

pub fn stop_search() -> Result<()> {
    if core_daemon::send_action(&DAEMON_CONFIG, "stop_search", true) {
        return Ok(());
    }
    bail!("Bluetooth daemon is not reachable")
}

fn discovery_state() -> Result<DiscoveryState> {
    DISCOVERY_STATE
        .read()
        .map(|state| state.clone())
        .map_err(|_| anyhow!("Bluetooth discovery state is unavailable"))
}

pub(super) fn searching() -> Result<bool> {
    discovery_state().map(|state| state.searching())
}

pub(super) fn mark_search_starting() -> Result<()> {
    DISCOVERY_STATE
        .write()
        .map(|mut state| state.start())
        .map_err(|_| anyhow!("Bluetooth discovery state is unavailable"))
}

pub(super) fn mark_search_stopped() -> Result<()> {
    DISCOVERY_STATE
        .write()
        .map(|mut state| state.stop())
        .map_err(|_| anyhow!("Bluetooth discovery state is unavailable"))
}

pub(super) fn reset_discovery_state() -> Result<()> {
    DISCOVERY_STATE
        .write()
        .map(|mut state| state.reset())
        .map_err(|_| anyhow!("Bluetooth discovery state is unavailable"))
}

pub(super) fn record_discovered_device(device: DeviceInfo) -> Result<()> {
    DISCOVERY_STATE
        .write()
        .map(|mut state| state.record_device(device))
        .map_err(|_| anyhow!("Bluetooth discovery state is unavailable"))
}

fn reconnect_candidates<S: PairedStack>(
    config: &ReconnectConfig,
    selection: ReconnectSelection,
) -> Vec<DeviceInfo> {
    let devices = S::paired_devices();
    match selection {
        ReconnectSelection::Trusted => devices.into_iter().filter(|item| item.paired).collect(),
        ReconnectSelection::Managed => {
            let managed = config
                .managed_devices
                .iter()
                .filter_map(|address| normalize_address(address).ok())
                .collect::<Vec<_>>();
            devices
                .into_iter()
                .filter(|item| managed.contains(&item.address))
                .collect()
        }
    }
}

pub(super) fn reconnect_devices<S: PairedStack>(
    config: &ReconnectConfig,
    selection: ReconnectSelection,
) -> Result<ReconnectReport> {
    ensure_powered::<S>(config.power_on_adapter)?;
    let mut report = ReconnectReport::default();
    for device in reconnect_candidates::<S>(config, selection) {
        if connection_ready(&device) {
            report.already_connected.push(device);
            continue;
        }
        match S::connect_device(&device.address, config.power_on_adapter) {
            Ok(connected) => report.connected.push(connected),
            Err(error) => report.failures.push(ReconnectFailure {
                address: device.address,
                alias: device.alias,
                error: format!("{error:#}"),
            }),
        }
    }
    qol_runtime::probe!(
        "BLUETOOTH_RECONNECT",
        "connected={} already={} failed={}",
        report.connected.len(),
        report.already_connected.len(),
        report.failures.len()
    );
    Ok(report)
}

pub(super) fn devices_snapshot<S: PairedStack>() -> Result<serde_json::Value> {
    let action = DEVICE_ACTION_STATE
        .read()
        .map_err(|_| anyhow!("Bluetooth device action state is unavailable"))?
        .clone();
    let payload = devices_payload(
        &S::paired_devices(),
        &crate::config::load().managed_devices,
        &discovery_state()?,
        action.as_ref(),
        super::CAPABILITIES,
    );
    qol_runtime::probe!(
        "BLUETOOTH_SNAPSHOT",
        "devices={} paired={} connected={} searching={}",
        payload["count"],
        payload["paired_count"],
        payload["connected_count"],
        payload["searching"]
    );
    Ok(payload)
}

pub fn search_status_snapshot() -> Result<serde_json::Value> {
    Ok(search_status_payload(&discovery_state()?))
}

fn adapter_status_snapshot<S: PairedStack>() -> Result<serde_json::Value> {
    let adapter = S::adapter_health()?;
    Ok(serde_json::json!({
        "available": true,
        "powered": adapter.powered,
    }))
}

fn current_managed_device_options<S: PairedStack>() -> Result<Vec<DeviceOption>> {
    Ok(managed_device_options(&S::paired_devices()))
}

fn current_adapter_options<S: PairedStack>() -> Result<Vec<DeviceOption>> {
    let health = S::adapter_health()?;
    Ok(adapter_options(&[AdapterInfo {
        name: health.name,
        address: health.address,
        paired_count: S::paired_devices().len(),
    }]))
}

pub fn settings_query(query: &str) -> std::result::Result<serde_json::Value, String> {
    match core_daemon::send_request(
        &DAEMON_CONFIG,
        query,
        serde_json::Value::Null,
        Duration::from_secs(2),
    ) {
        Ok(DaemonResponse::Handled { data: Some(data) }) => Ok(data),
        Ok(DaemonResponse::Handled { data: None }) => Ok(serde_json::Value::Null),
        Ok(DaemonResponse::Error { message }) => Err(message),
        Ok(DaemonResponse::Fallback) => Err("Bluetooth daemon declined the query".into()),
        Ok(DaemonResponse::NotReady { .. }) => Err("Bluetooth daemon is still starting".into()),
        Err(error) => Err(format!("Bluetooth daemon query failed: {error}")),
    }
}

pub fn settings_action(action: &str, input: serde_json::Value) -> std::result::Result<(), String> {
    match core_daemon::send_request(&DAEMON_CONFIG, action, input, Duration::from_secs(2)) {
        Ok(DaemonResponse::Handled { .. }) => Ok(()),
        Ok(DaemonResponse::Error { message }) => Err(message),
        Ok(DaemonResponse::Fallback) => Err("Bluetooth daemon declined the action".into()),
        Ok(DaemonResponse::NotReady { .. }) => Err("Bluetooth daemon is still starting".into()),
        Err(error) => Err(format!("Bluetooth daemon action failed: {error}")),
    }
}

enum DaemonCommand {
    Kill,
    SetAdapterPower(bool),
    Pair(String),
    Connect(String),
    Disconnect(String),
    Remove(String),
    StartSearch,
    StopSearch,
    ReconnectManaged,
    ReconnectTrusted,
    Reload,
    Settings,
}

fn parse_daemon_request<S: PairedStack>(request: &DaemonRequest) -> ReadResult<DaemonCommand> {
    match request.action.as_str() {
        "ping" => ReadResult::Handled,
        "kill" => ReadResult::Command(DaemonCommand::Kill),
        "enable_adapter" => ReadResult::Command(DaemonCommand::SetAdapterPower(true)),
        "disable_adapter" => ReadResult::Command(DaemonCommand::SetAdapterPower(false)),
        "pair_device" => device_daemon_command(request, DaemonCommand::Pair, DeviceIntent::Pair),
        "connect_device" => {
            device_daemon_command(request, DaemonCommand::Connect, DeviceIntent::Connect)
        }
        "disconnect_device" => {
            device_daemon_command(request, DaemonCommand::Disconnect, DeviceIntent::Disconnect)
        }
        "reclaim_device" => reclaim_command(request),
        "remove_device" => {
            device_daemon_command(request, DaemonCommand::Remove, DeviceIntent::Remove)
        }
        "trust_device" | "untrust_device" => ReadResult::Error(S::TRUST_REFUSAL.into()),
        "start_search" => ReadResult::Command(DaemonCommand::StartSearch),
        "stop_search" => match mark_search_stopped() {
            Ok(()) => ReadResult::Command(DaemonCommand::StopSearch),
            Err(error) => ReadResult::Error(error.to_string()),
        },
        "devices" => snapshot_result(devices_snapshot::<S>()),
        "search_status" => snapshot_result(search_status_snapshot()),
        "adapter_status" => snapshot_result(adapter_status_snapshot::<S>()),
        "managed_device_options" => {
            snapshot_result(current_managed_device_options::<S>().and_then(|options| {
                serde_json::to_value(options).context("failed to encode device options")
            }))
        }
        "adapter_options" => snapshot_result(current_adapter_options::<S>().and_then(|options| {
            serde_json::to_value(options).context("failed to encode adapter options")
        })),
        "reconnect" => ReadResult::Command(DaemonCommand::ReconnectManaged),
        "reconnect_trusted" => ReadResult::Command(DaemonCommand::ReconnectTrusted),
        "reload" => ReadResult::Command(DaemonCommand::Reload),
        "settings" => ReadResult::Command(DaemonCommand::Settings),
        "host_fixes" => ReadResult::HandledWithData(findings_payload(&BluetoothHostFixes.detect())),
        "apply_host_fix" => match host_fix_id(request) {
            Ok(id) => {
                spawn_host_fix(id);
                ReadResult::Handled
            }
            Err(message) => ReadResult::Error(message),
        },
        "handoff_state" => handoff_result(request, |address| {
            Ok(crate::handoff::state(address, &S::paired_devices()))
        }),
        "release_for_handoff" => handoff_result(request, |address| {
            crate::handoff::release(
                address,
                || Ok(S::paired_devices()),
                S::disconnect_device,
                S::pause,
            )
        }),
        "resume_reconnect" => handoff_result(request, crate::handoff::resume),
        unknown => ReadResult::Error(format!("unknown Bluetooth action: {unknown}")),
    }
}

fn handoff_result(
    request: &DaemonRequest,
    operation: impl FnOnce(&str) -> Result<serde_json::Value>,
) -> ReadResult<DaemonCommand> {
    match request_address(request) {
        Ok(address) => snapshot_result(operation(&address)),
        Err(error) => ReadResult::Error(error),
    }
}

fn snapshot_result(payload: Result<serde_json::Value>) -> ReadResult<DaemonCommand> {
    match payload {
        Ok(payload) => ReadResult::HandledWithData(payload),
        Err(error) => ReadResult::Error(format!("{error:#}")),
    }
}

fn device_daemon_command(
    request: &DaemonRequest,
    command: fn(String) -> DaemonCommand,
    intent: DeviceIntent,
) -> ReadResult<DaemonCommand> {
    match request_address(request) {
        Ok(address) => match begin_device_action(&address, intent) {
            Ok(()) => ReadResult::Command(command(address)),
            Err(error) => ReadResult::Error(error.to_string()),
        },
        Err(error) => ReadResult::Error(error),
    }
}

fn request_address(request: &DaemonRequest) -> std::result::Result<String, String> {
    let Some(address) = request
        .input
        .get("address")
        .and_then(serde_json::Value::as_str)
    else {
        return Err(format!("{} requires an address", request.action));
    };
    normalize_address(address).map_err(|error| error.to_string())
}

fn reclaim_command(request: &DaemonRequest) -> ReadResult<DaemonCommand> {
    match request_address(request) {
        Ok(address) => match crate::audio_claim::platform::reclaim_output(&address) {
            Ok(()) => ReadResult::Handled,
            Err(error) => ReadResult::Error(format!("{error:#}")),
        },
        Err(error) => ReadResult::Error(error),
    }
}

fn host_fix_id(request: &DaemonRequest) -> std::result::Result<String, String> {
    request
        .input
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "apply_host_fix requires an id".to_string())
}

fn spawn_host_fix(id: String) {
    std::mem::drop(std::thread::spawn(move || {
        match BluetoothHostFixes.apply(&id) {
            Ok(message) => {
                qol_runtime::probe!("BLUETOOTH_HOST_FIX", "stage=apply fix={id} outcome=ok");
                send_notification("Bluetooth", &message);
            }
            Err(error) => {
                qol_runtime::probe!("BLUETOOTH_HOST_FIX", "stage=apply fix={id} outcome=failed");
                log::warn!("Bluetooth host fix {id} failed: {error:#}");
                send_notification("Bluetooth", &format!("{error:#}"));
            }
        }
    }));
}

fn set_device_action_state(action: Option<DeviceActionState>) {
    if let Ok(mut state) = DEVICE_ACTION_STATE.write() {
        *state = action;
    }
}

fn begin_device_action(address: &str, intent: DeviceIntent) -> Result<()> {
    let mut state = DEVICE_ACTION_STATE
        .write()
        .map_err(|_| anyhow!("Bluetooth device action state is unavailable"))?;
    if state.as_ref().is_some_and(|action| action.pending) {
        bail!("another Bluetooth device action is already running");
    }
    *state = Some(DeviceActionState {
        address: address.to_string(),
        intent,
        status: intent.pending_status().to_string(),
        pending: true,
    });
    Ok(())
}

fn finish_device_action(address: &str, label: &str, result: &Result<()>) {
    match result {
        Ok(()) => set_device_action_state(None),
        Err(error) => {
            log::warn!("Bluetooth {label} failed for {address}: {error:#}");
            let Ok(mut state) = DEVICE_ACTION_STATE.write() else {
                return;
            };
            if let Some(action) = state.as_mut() {
                action.status = format!("{error:#}");
                action.pending = false;
            }
        }
    }
}

fn run_device_command(address: &str, label: &str, action: impl FnOnce() -> Result<()>) {
    let result = action();
    let outcome = if result.is_ok() { "ok" } else { "failed" };
    qol_runtime::probe!(
        "BLUETOOTH_DEVICE_ACTION",
        "action={label} outcome={outcome}"
    );
    finish_device_action(address, label, &result);
}

fn report_daemon_failure(label: &str, result: Result<()>) {
    if let Err(error) = result {
        log::warn!("Bluetooth {label} failed: {error:#}");
    }
}

fn handle_daemon_command<S: PairedStack>(
    command: DaemonCommand,
    config: &mut ReconnectConfig,
) -> bool {
    let power_on_adapter = config.power_on_adapter;
    match command {
        DaemonCommand::Kill => return false,
        DaemonCommand::Reload => *config = crate::config::load(),
        DaemonCommand::SetAdapterPower(powered) => report_daemon_failure(
            "adapter power change",
            S::set_adapter_powered(powered).map(std::mem::drop),
        ),
        DaemonCommand::Pair(address) => run_device_command(&address, "pair", || {
            S::pair_device(&address, power_on_adapter).map(std::mem::drop)
        }),
        DaemonCommand::Connect(address) => run_device_command(&address, "connect", || {
            crate::connect::for_user(&address, power_on_adapter, |address| {
                S::connect_device(address, power_on_adapter)
            })
            .map(std::mem::drop)
        }),
        DaemonCommand::Disconnect(address) => run_device_command(&address, "disconnect", || {
            S::disconnect_device(&address).map(std::mem::drop)
        }),
        DaemonCommand::Remove(address) => {
            run_device_command(&address, "remove", || S::remove_device(&address))
        }
        DaemonCommand::StartSearch => {
            if S::search_devices(config).is_err() {
                report_daemon_failure("search", reset_discovery_state());
            }
        }
        DaemonCommand::StopSearch => report_daemon_failure("search stop", mark_search_stopped()),
        DaemonCommand::ReconnectManaged | DaemonCommand::ReconnectTrusted => {
            crate::handoff::release_managed_for_user(config);
            let selection = if matches!(command, DaemonCommand::ReconnectTrusted) {
                ReconnectSelection::Trusted
            } else {
                ReconnectSelection::Managed
            };
            report_daemon_failure(
                "reconnect",
                reconnect_devices::<S>(config, selection).map(std::mem::drop),
            )
        }
        DaemonCommand::Settings => {
            report_daemon_failure("settings", crate::settings::open_browser())
        }
    }
    true
}

fn run_retry_pass<S: PairedStack>(
    config: &ReconnectConfig,
    retries: &mut HashMap<String, RetryState>,
    now: Instant,
) {
    if !config.auto_reconnect {
        return;
    }
    let policy = RetryPolicy::from_seconds(config.retry_initial_seconds, config.retry_max_seconds);
    for device in reconnect_candidates::<S>(config, ReconnectSelection::Managed) {
        let state = retries.entry(device.address.clone()).or_default();
        if connection_ready(&device) {
            state.connected();
            continue;
        }
        state.request_when_idle(now);
        if !state.is_due(now) || crate::handoff::held(&device.address) {
            continue;
        }
        match S::connect_device(&device.address, config.power_on_adapter) {
            Ok(_) => state.connected(),
            Err(error) => {
                let delay = state.failed(now, policy);
                qol_runtime::probe!(
                    "BLUETOOTH_RETRY",
                    "failures={} delay_ms={} error={error:#}",
                    state.failures(),
                    delay.as_millis()
                );
            }
        }
    }
}

pub(super) fn run_daemon<S: PairedStack>(mut config: ReconnectConfig) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    if !core_daemon::start_request_listener(&DAEMON_CONFIG, tx, parse_daemon_request::<S>) {
        bail!("{} daemon listener failed to start", crate::PLUGIN_ID);
    }

    let mut retries: HashMap<String, RetryState> = HashMap::new();
    let mut retried: Option<Instant> = None;
    loop {
        S::pause(DAEMON_TICK);
        while let Ok(command) = rx.try_recv() {
            if !handle_daemon_command::<S>(command, &mut config) {
                return Ok(());
            }
        }
        let now = Instant::now();
        if retried.is_none_or(|last| now.duration_since(last) >= S::RETRY_SPACING) {
            run_retry_pass::<S>(&config, &mut retries, now);
            retried = Some(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REFUSAL: &str = "this stack pairs and trusts in one step";

    struct Fake;

    impl PairedStack for Fake {
        const TRUST_REFUSAL: &'static str = REFUSAL;
        const RETRY_SPACING: Duration = Duration::ZERO;

        fn paired_devices() -> Vec<DeviceInfo> {
            vec![device("AA:BB:CC:DD:EE:01")]
        }

        fn adapter_health() -> Result<AdapterHealth> {
            bail!("no adapter in tests")
        }

        fn set_adapter_powered(_powered: bool) -> Result<AdapterHealth> {
            bail!("no adapter in tests")
        }

        fn connect_device(_address: &str, _power_on_adapter: bool) -> Result<DeviceInfo> {
            bail!("no adapter in tests")
        }

        fn disconnect_device(_address: &str) -> Result<DeviceInfo> {
            bail!("no adapter in tests")
        }

        fn pair_device(_address: &str, _power_on_adapter: bool) -> Result<DeviceInfo> {
            bail!("no adapter in tests")
        }

        fn remove_device(_address: &str) -> Result<()> {
            bail!("no adapter in tests")
        }

        fn search_devices(_config: &ReconnectConfig) -> Result<Vec<DeviceInfo>> {
            bail!("no adapter in tests")
        }

        fn pause(_slice: Duration) {}
    }

    fn device(address: &str) -> DeviceInfo {
        DeviceInfo {
            address: address.into(),
            alias: "Luna 2".into(),
            paired: true,
            trusted: true,
            connected: false,
            audio_connected: None,
            services_resolved: false,
            icon: None,
            class: None,
            uuids: Vec::new(),
            rssi: None,
        }
    }

    fn test_config(managed: &[&str]) -> ReconnectConfig {
        ReconnectConfig {
            adapter: String::new(),
            managed_devices: managed.iter().map(|item| (*item).to_string()).collect(),
            auto_reconnect: true,
            power_on_adapter: true,
            set_default_output: true,
            retry_initial_seconds: 1.0,
            retry_max_seconds: 60.0,
        }
    }

    fn request(action: &str) -> DaemonRequest {
        DaemonRequest {
            action: action.to_string(),
            input: serde_json::Value::Null,
        }
    }

    #[test]
    fn the_paired_surface_offers_no_trust_row_for_a_paired_device() {
        let payload = devices_payload(
            &[device("AA:BB:CC:DD:EE:FF")],
            &[],
            &DiscoveryState::default(),
            None,
            super::super::CAPABILITIES,
        );

        let item = &payload["items"][0];
        assert_eq!(item["can_trust"], false);
        assert_eq!(item["can_untrust"], false);
        assert_eq!(item["can_connect"], true);
        assert_eq!(item["can_remove"], true);
    }

    #[test]
    fn managed_reconnect_keeps_only_configured_devices() {
        let cases = [
            (&[][..], ReconnectSelection::Managed, 0),
            (&["aa:bb:cc:dd:ee:01"][..], ReconnectSelection::Managed, 1),
            (&["aa:bb:cc:dd:ee:02"][..], ReconnectSelection::Managed, 0),
            (&[][..], ReconnectSelection::Trusted, 1),
        ];
        for (managed, selection, expected) in cases {
            let candidates = reconnect_candidates::<Fake>(&test_config(managed), selection);
            assert_eq!(candidates.len(), expected, "{managed:?} {selection:?}");
        }
    }

    #[test]
    fn unknown_daemon_actions_are_rejected_by_name() {
        match parse_daemon_request::<Fake>(&request("not_a_bluetooth_action")) {
            ReadResult::Error(message) => assert!(message.contains("not_a_bluetooth_action")),
            _ => panic!("unknown actions must be rejected"),
        }
    }

    #[test]
    fn trust_actions_are_refused_with_the_stack_reason() {
        for action in ["trust_device", "untrust_device"] {
            match parse_daemon_request::<Fake>(&request(action)) {
                ReadResult::Error(message) => assert_eq!(message, REFUSAL, "{action}"),
                _ => panic!("{action} must be refused"),
            }
        }
    }

    #[test]
    fn device_actions_require_an_address() {
        for action in [
            "pair_device",
            "connect_device",
            "disconnect_device",
            "remove_device",
        ] {
            match parse_daemon_request::<Fake>(&request(action)) {
                ReadResult::Error(message) => assert!(message.contains("requires an address")),
                _ => panic!("{action} must require an address"),
            }
        }
    }
}
