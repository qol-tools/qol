use std::collections::HashMap;
use std::ffi::CString;
use std::sync::{mpsc, LazyLock, RwLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use objc2::rc::Retained;
use objc2_core_foundation::{kCFRunLoopDefaultMode, CFRunLoop};
use objc2_foundation::{NSArray, NSString};
use objc2_io_bluetooth::{
    BluetoothHCIPowerState, IOBluetoothDevice, IOBluetoothDeviceInquiry, IOBluetoothDevicePair,
    IOBluetoothHostController, IOBluetoothSDPUUID,
};
use qol_headless::DoctorCheckResult;
use qol_host_fixes::{findings_payload, HostFixes};
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_plugin_daemon::notification::send_notification;
use qol_runtime::protocol::{DaemonRequest, DaemonResponse};

use crate::bluetooth::{
    adapter_options, connection_ready, devices_payload, managed_device_options, normalize_address,
    retry::{RetryPolicy, RetryState},
    search_status_payload, AdapterHealth, AdapterInfo, BackendCapabilities, DeviceActionState,
    DeviceInfo, DeviceOption, DiscoveryState, ReconnectFailure, ReconnectReport,
    ReconnectSelection,
};
use crate::config::ReconnectConfig;
use crate::hostfix::BluetoothHostFixes;

pub const CAPABILITIES: BackendCapabilities = BackendCapabilities {
    separate_trust_flag: false,
};

const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

static DISCOVERY_STATE: LazyLock<RwLock<DiscoveryState>> =
    LazyLock::new(|| RwLock::new(DiscoveryState::default()));
static DEVICE_ACTION_STATE: LazyLock<RwLock<Option<DeviceActionState>>> =
    LazyLock::new(|| RwLock::new(None));

const AUDIO_SINK_UUID16: u16 = 0x110b;
const AUDIO_SINK_UUID: &str = "0000110b-0000-1000-8000-00805f9b34fb";
const IO_RETURN_SUCCESS: i32 = 0;
const RSSI_UNAVAILABLE: i8 = 127;
const SEARCH_SECONDS: u8 = 10;
const PAIR_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const RUN_LOOP_SLICE: Duration = Duration::from_millis(100);
const DAEMON_TICK: Duration = Duration::from_millis(250);
const ADAPTER_POWER_SYMBOL: &str = "IOBluetoothPreferenceSetControllerPowerState";

fn deliver_pending_iobluetooth_callbacks(slice: Duration) {
    let mode = unsafe { kCFRunLoopDefaultMode };
    CFRunLoop::run_in_mode(mode, slice.as_secs_f64(), false);
}

fn deliver_callbacks_until(timeout: Duration, mut satisfied: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if satisfied() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        deliver_pending_iobluetooth_callbacks(RUN_LOOP_SLICE);
    }
}

fn set_controller_power_state(powered: bool) -> Result<()> {
    type SetPowerState = unsafe extern "C" fn(i32) -> i32;

    let name = CString::new(ADAPTER_POWER_SYMBOL).expect("symbol name has no interior nul");
    let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
    if symbol.is_null() {
        bail!("this macOS build exposes no adapter power control for Bluetooth");
    }
    let set_power_state =
        unsafe { std::mem::transmute::<*mut libc::c_void, SetPowerState>(symbol) };
    let result = unsafe { set_power_state(i32::from(powered)) };
    if result != IO_RETURN_SUCCESS {
        bail!("macOS refused to change the Bluetooth adapter power state (code {result})");
    }
    Ok(())
}

fn remove_pairing(device: &IOBluetoothDevice) -> Result<()> {
    let selector = objc2::sel!(remove);
    let exposes_unpair: bool = unsafe { objc2::msg_send![device, respondsToSelector: selector] };
    if !exposes_unpair {
        bail!("this macOS build exposes no unpair operation for Bluetooth devices");
    }
    let result: i32 = unsafe { objc2::msg_send![device, remove] };
    if result != IO_RETURN_SUCCESS {
        bail!("macOS refused to unpair the device (code {result})");
    }
    Ok(())
}

fn host_controller() -> Result<Retained<IOBluetoothHostController>> {
    unsafe { IOBluetoothHostController::defaultController() }
        .ok_or_else(|| anyhow!("this Mac has no Bluetooth controller"))
}

fn controller_is_powered(controller: &IOBluetoothHostController) -> bool {
    let state: i32 = unsafe { objc2::msg_send![controller, powerState] };
    state == BluetoothHCIPowerState::ON.0 as i32
}

fn colon_separated_address(iobluetooth_address: &str) -> String {
    iobluetooth_address.replace('-', ":").to_ascii_uppercase()
}

fn device_address(device: &IOBluetoothDevice) -> Result<String> {
    let raw = unsafe { device.addressString() }
        .ok_or_else(|| anyhow!("a Bluetooth device reported no address"))?;
    normalize_address(&colon_separated_address(&raw.to_string()))
}

fn device_alias(device: &IOBluetoothDevice, address: &str) -> String {
    unsafe { device.nameOrAddress() }
        .map(|name| name.to_string())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| address.to_string())
}

fn cached_audio_sink_service(device: &IOBluetoothDevice) -> (bool, Vec<String>) {
    let Some(uuid) = (unsafe { IOBluetoothSDPUUID::uuid16(AUDIO_SINK_UUID16) }) else {
        return (false, Vec::new());
    };
    let services_resolved = unsafe { device.services() }.is_some();
    let advertises_sink = unsafe { device.getServiceRecordForUUID(Some(&uuid)) }.is_some();
    let uuids = if advertises_sink {
        vec![AUDIO_SINK_UUID.to_string()]
    } else {
        Vec::new()
    };
    (services_resolved, uuids)
}

fn device_info(device: &IOBluetoothDevice) -> Result<DeviceInfo> {
    let address = device_address(device)?;
    let alias = device_alias(device, &address);
    let paired = unsafe { device.isPaired() };
    let (services_resolved, uuids) = cached_audio_sink_service(device);
    let rssi = unsafe { device.RSSI() };
    Ok(DeviceInfo {
        alias,
        trusted: paired,
        paired,
        connected: unsafe { device.isConnected() },
        services_resolved,
        icon: None,
        class: Some(unsafe { device.classOfDevice() }),
        uuids,
        rssi: (rssi != RSSI_UNAVAILABLE).then_some(i16::from(rssi)),
        address,
    })
}

fn richer_duplicate(existing: &DeviceInfo, candidate: &DeviceInfo) -> bool {
    (candidate.connected, candidate.paired, candidate.uuids.len())
        > (existing.connected, existing.paired, existing.uuids.len())
}

fn readable_device_infos(devices: Option<Retained<NSArray>>) -> Vec<DeviceInfo> {
    let Some(devices) = devices else {
        return Vec::new();
    };
    let mut ordered: Vec<DeviceInfo> = Vec::new();
    for object in devices.iter() {
        let device = unsafe { Retained::cast_unchecked::<IOBluetoothDevice>(object) };
        let Ok(info) = device_info(&device) else {
            continue;
        };
        match ordered
            .iter_mut()
            .find(|existing| existing.address == info.address)
        {
            Some(existing) => {
                if richer_duplicate(existing, &info) {
                    *existing = info;
                }
            }
            None => ordered.push(info),
        }
    }
    ordered
}

fn paired_devices() -> Vec<DeviceInfo> {
    readable_device_infos(unsafe { IOBluetoothDevice::pairedDevices() })
}

fn device_handle(address: &str) -> Result<Retained<IOBluetoothDevice>> {
    let address = normalize_address(address)?;
    let text = NSString::from_str(&address);
    unsafe { IOBluetoothDevice::deviceWithAddressString(Some(&text)) }
        .ok_or_else(|| anyhow!("macOS knows no Bluetooth device at {address}"))
}

pub fn required_binaries_check() -> DoctorCheckResult {
    fn details(controller: bool, powered: Option<bool>) -> serde_json::Value {
        serde_json::json!({
            "platform": "macos",
            "controller": controller,
            "powered": powered,
            "executed": false,
        })
    }

    let Some(controller) = (unsafe { IOBluetoothHostController::defaultController() }) else {
        return DoctorCheckResult::fail(
            "required_binaries",
            "macOS reported no default Bluetooth controller",
        )
        .with_fix("Check that this Mac has Bluetooth hardware and that it is not disabled")
        .with_details(details(false, None));
    };
    let powered = controller_is_powered(&controller);
    DoctorCheckResult::ok(
        "required_binaries",
        "The macOS IOBluetooth controller is available",
    )
    .with_details(details(true, Some(powered)))
}

pub fn list_devices() -> Result<Vec<DeviceInfo>> {
    Ok(paired_devices())
}

pub fn adapter_health() -> Result<AdapterHealth> {
    let controller = host_controller()?;
    let name = unsafe { controller.nameAsString() }
        .map(|name| name.to_string())
        .unwrap_or_else(|| "Bluetooth".to_string());
    let address = unsafe { controller.addressAsString() }
        .map(|address| colon_separated_address(&address.to_string()))
        .unwrap_or_default();
    Ok(AdapterHealth {
        name,
        address,
        powered: controller_is_powered(&controller),
    })
}

pub fn set_adapter_powered(powered: bool) -> Result<AdapterHealth> {
    let result = set_controller_power_state(powered);
    let outcome = if result.is_ok() { "ok" } else { "failed" };
    qol_runtime::probe!(
        "BLUETOOTH_ADAPTER_POWER",
        "source=cli powered={powered} outcome={outcome}"
    );
    result?;
    deliver_callbacks_until(CONNECT_TIMEOUT, || {
        adapter_health().is_ok_and(|health| health.powered == powered)
    });
    adapter_health()
}

fn ensure_powered(power_on_adapter: bool) -> Result<()> {
    if adapter_health()?.powered {
        return Ok(());
    }
    if !power_on_adapter {
        bail!("the Bluetooth adapter is off");
    }
    set_adapter_powered(true)?;
    Ok(())
}

pub fn connect_device(address: &str, power_on_adapter: bool) -> Result<DeviceInfo> {
    ensure_powered(power_on_adapter)?;
    let device = device_handle(address)?;
    if unsafe { device.isConnected() } {
        return device_info(&device);
    }
    let result = unsafe { device.openConnection() };
    if result != IO_RETURN_SUCCESS {
        bail!("macOS could not connect to {address} (code {result})");
    }
    if !deliver_callbacks_until(CONNECT_TIMEOUT, || unsafe { device.isConnected() }) {
        bail!("{address} did not finish connecting");
    }
    device_info(&device)
}

pub fn disconnect_device(address: &str) -> Result<DeviceInfo> {
    let device = device_handle(address)?;
    let result = unsafe { device.closeConnection() };
    if result != IO_RETURN_SUCCESS {
        bail!("macOS could not disconnect {address} (code {result})");
    }
    device_info(&device)
}

pub fn pair_device(address: &str, power_on_adapter: bool) -> Result<DeviceInfo> {
    ensure_powered(power_on_adapter)?;
    let device = device_handle(address)?;
    if unsafe { device.isPaired() } {
        return device_info(&device);
    }
    let pairing = unsafe { IOBluetoothDevicePair::pairWithDevice(Some(&device)) }
        .ok_or_else(|| anyhow!("macOS could not start pairing with {address}"))?;
    let started = unsafe { pairing.start() };
    if started != IO_RETURN_SUCCESS {
        bail!("macOS refused to pair with {address} (code {started})");
    }
    let paired = deliver_callbacks_until(PAIR_TIMEOUT, || unsafe { device.isPaired() });
    unsafe { pairing.stop() };
    if !paired {
        bail!("pairing with {address} did not complete");
    }
    device_info(&device)
}

pub fn set_device_trusted(_address: &str, _trusted: bool) -> Result<DeviceInfo> {
    bail!("macOS pairs and trusts in one step, so there is no separate trust to change")
}

pub fn remove_device(address: &str) -> Result<()> {
    let device = device_handle(address)?;
    remove_pairing(&device)
}

pub fn search_devices(config: &ReconnectConfig) -> Result<Vec<DeviceInfo>> {
    ensure_powered(config.power_on_adapter)?;
    mark_search_starting()?;

    let inquiry = unsafe { IOBluetoothDeviceInquiry::inquiryWithDelegate(None) }
        .ok_or_else(|| anyhow!("macOS could not start a Bluetooth search"))?;
    unsafe {
        inquiry.setInquiryLength(SEARCH_SECONDS);
        inquiry.setUpdateNewDeviceNames(true);
    }
    let started = unsafe { inquiry.start() };
    if started != IO_RETURN_SUCCESS {
        reset_discovery_state()?;
        bail!("macOS refused to start a Bluetooth search (code {started})");
    }

    let deadline = Instant::now() + Duration::from_secs(u64::from(SEARCH_SECONDS));
    while Instant::now() < deadline && searching()? {
        deliver_pending_iobluetooth_callbacks(RUN_LOOP_SLICE);
        for device in readable_device_infos(unsafe { inquiry.foundDevices() }) {
            record_discovered_device(device)?;
        }
    }
    unsafe { inquiry.stop() };
    mark_search_stopped()?;

    let found = readable_device_infos(unsafe { inquiry.foundDevices() });
    qol_runtime::probe!("BLUETOOTH_SEARCH", "stage=stop found={}", found.len());
    Ok(found)
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

fn searching() -> Result<bool> {
    discovery_state().map(|state| state.searching())
}

fn mark_search_starting() -> Result<()> {
    DISCOVERY_STATE
        .write()
        .map(|mut state| state.start())
        .map_err(|_| anyhow!("Bluetooth discovery state is unavailable"))
}

fn mark_search_stopped() -> Result<()> {
    DISCOVERY_STATE
        .write()
        .map(|mut state| state.stop())
        .map_err(|_| anyhow!("Bluetooth discovery state is unavailable"))
}

fn reset_discovery_state() -> Result<()> {
    DISCOVERY_STATE
        .write()
        .map(|mut state| state.reset())
        .map_err(|_| anyhow!("Bluetooth discovery state is unavailable"))
}

fn record_discovered_device(device: DeviceInfo) -> Result<()> {
    DISCOVERY_STATE
        .write()
        .map(|mut state| state.record_device(device))
        .map_err(|_| anyhow!("Bluetooth discovery state is unavailable"))
}

fn reconnect_candidates(
    config: &ReconnectConfig,
    selection: ReconnectSelection,
) -> Vec<DeviceInfo> {
    let devices = paired_devices();
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

pub fn reconnect_devices(
    config: &ReconnectConfig,
    selection: ReconnectSelection,
) -> Result<ReconnectReport> {
    ensure_powered(config.power_on_adapter)?;
    let mut report = ReconnectReport::default();
    for device in reconnect_candidates(config, selection) {
        if connection_ready(&device) {
            report.already_connected.push(device);
            continue;
        }
        match connect_device(&device.address, config.power_on_adapter) {
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

pub fn devices_snapshot() -> Result<serde_json::Value> {
    let action = DEVICE_ACTION_STATE
        .read()
        .map_err(|_| anyhow!("Bluetooth device action state is unavailable"))?
        .clone();
    let payload = devices_payload(
        &paired_devices(),
        &crate::config::load().managed_devices,
        &discovery_state()?,
        action.as_ref(),
        CAPABILITIES,
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

fn adapter_status_snapshot() -> Result<serde_json::Value> {
    let adapter = adapter_health().ok();
    Ok(serde_json::json!({
        "available": adapter.is_some(),
        "powered": adapter.is_some_and(|adapter| adapter.powered),
    }))
}

fn current_managed_device_options() -> Result<Vec<DeviceOption>> {
    Ok(managed_device_options(&paired_devices()))
}

fn current_adapter_options() -> Result<Vec<DeviceOption>> {
    let health = adapter_health()?;
    Ok(adapter_options(&[AdapterInfo {
        name: health.name,
        address: health.address,
        paired_count: paired_devices().len(),
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

const TRUST_UNSUPPORTED: &str =
    "macOS pairs and trusts in one step, so there is no separate trust to change";

fn parse_daemon_request(request: &DaemonRequest) -> ReadResult<DaemonCommand> {
    match request.action.as_str() {
        "ping" => ReadResult::Handled,
        "kill" => ReadResult::Command(DaemonCommand::Kill),
        "enable_adapter" => ReadResult::Command(DaemonCommand::SetAdapterPower(true)),
        "disable_adapter" => ReadResult::Command(DaemonCommand::SetAdapterPower(false)),
        "pair_device" => device_daemon_command(request, DaemonCommand::Pair, "Pairing"),
        "connect_device" => device_daemon_command(request, DaemonCommand::Connect, "Connecting"),
        "disconnect_device" => {
            device_daemon_command(request, DaemonCommand::Disconnect, "Disconnecting")
        }
        "remove_device" => device_daemon_command(request, DaemonCommand::Remove, "Removing"),
        "trust_device" | "untrust_device" => ReadResult::Error(TRUST_UNSUPPORTED.into()),
        "start_search" => ReadResult::Command(DaemonCommand::StartSearch),
        "stop_search" => match mark_search_stopped() {
            Ok(()) => ReadResult::Command(DaemonCommand::StopSearch),
            Err(error) => ReadResult::Error(error.to_string()),
        },
        "devices" => snapshot_result(devices_snapshot()),
        "search_status" => snapshot_result(search_status_snapshot()),
        "adapter_status" => snapshot_result(adapter_status_snapshot()),
        "managed_device_options" => {
            snapshot_result(current_managed_device_options().and_then(|options| {
                serde_json::to_value(options).context("failed to encode device options")
            }))
        }
        "adapter_options" => snapshot_result(current_adapter_options().and_then(|options| {
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
        unknown => ReadResult::Error(format!("unknown Bluetooth action: {unknown}")),
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
    pending_status: &str,
) -> ReadResult<DaemonCommand> {
    let Some(address) = request
        .input
        .get("address")
        .and_then(serde_json::Value::as_str)
    else {
        return ReadResult::Error(format!("{} requires an address", request.action));
    };
    match normalize_address(address) {
        Ok(address) => match begin_device_action(&address, pending_status) {
            Ok(()) => ReadResult::Command(command(address)),
            Err(error) => ReadResult::Error(error.to_string()),
        },
        Err(error) => ReadResult::Error(error.to_string()),
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
                eprintln!("Bluetooth host fix {id} failed: {error:#}");
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

fn begin_device_action(address: &str, status: &str) -> Result<()> {
    let mut state = DEVICE_ACTION_STATE
        .write()
        .map_err(|_| anyhow!("Bluetooth device action state is unavailable"))?;
    if state.as_ref().is_some_and(|action| action.pending) {
        bail!("another Bluetooth device action is already running");
    }
    *state = Some(DeviceActionState {
        address: address.to_string(),
        status: status.to_string(),
        pending: true,
    });
    Ok(())
}

fn finish_device_action(address: &str, label: &str, result: &Result<()>) {
    match result {
        Ok(()) => set_device_action_state(None),
        Err(error) => {
            eprintln!("Bluetooth {label} failed for {address}: {error:#}");
            set_device_action_state(Some(DeviceActionState {
                address: address.to_string(),
                status: format!("{error:#}"),
                pending: false,
            }));
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
        eprintln!("Bluetooth {label} failed: {error:#}");
    }
}

fn handle_daemon_command(command: DaemonCommand, config: &mut ReconnectConfig) -> bool {
    let power_on_adapter = config.power_on_adapter;
    match command {
        DaemonCommand::Kill => return false,
        DaemonCommand::Reload => *config = crate::config::load(),
        DaemonCommand::SetAdapterPower(powered) => report_daemon_failure(
            "adapter power change",
            set_adapter_powered(powered).map(std::mem::drop),
        ),
        DaemonCommand::Pair(address) => run_device_command(&address, "pair", || {
            pair_device(&address, power_on_adapter).map(std::mem::drop)
        }),
        DaemonCommand::Connect(address) => run_device_command(&address, "connect", || {
            connect_device(&address, power_on_adapter).map(std::mem::drop)
        }),
        DaemonCommand::Disconnect(address) => run_device_command(&address, "disconnect", || {
            disconnect_device(&address).map(std::mem::drop)
        }),
        DaemonCommand::Remove(address) => {
            run_device_command(&address, "remove", || remove_device(&address))
        }
        DaemonCommand::StartSearch => {
            if search_devices(config).is_err() {
                report_daemon_failure("search", reset_discovery_state());
            }
        }
        DaemonCommand::StopSearch => report_daemon_failure("search stop", mark_search_stopped()),
        DaemonCommand::ReconnectManaged => report_daemon_failure(
            "reconnect",
            reconnect_devices(config, ReconnectSelection::Managed).map(std::mem::drop),
        ),
        DaemonCommand::ReconnectTrusted => report_daemon_failure(
            "reconnect",
            reconnect_devices(config, ReconnectSelection::Trusted).map(std::mem::drop),
        ),
        DaemonCommand::Settings => {
            report_daemon_failure("settings", crate::settings::open_browser())
        }
    }
    true
}

fn run_retry_pass(
    config: &ReconnectConfig,
    retries: &mut HashMap<String, RetryState>,
    now: Instant,
) {
    if !config.auto_reconnect {
        return;
    }
    let policy = RetryPolicy::from_seconds(config.retry_initial_seconds, config.retry_max_seconds);
    for device in reconnect_candidates(config, ReconnectSelection::Managed) {
        let state = retries.entry(device.address.clone()).or_default();
        if connection_ready(&device) {
            state.connected();
            continue;
        }
        state.request_when_idle(now);
        if !state.is_due(now) {
            continue;
        }
        match connect_device(&device.address, config.power_on_adapter) {
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

pub fn run_daemon(mut config: ReconnectConfig) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    if !core_daemon::start_request_listener(&DAEMON_CONFIG, tx, parse_daemon_request) {
        bail!("plugin-bluetooth daemon listener failed to start");
    }

    let mut retries: HashMap<String, RetryState> = HashMap::new();
    loop {
        deliver_pending_iobluetooth_callbacks(DAEMON_TICK);
        while let Ok(command) = rx.try_recv() {
            if !handle_daemon_command(command, &mut config) {
                return Ok(());
            }
        }
        run_retry_pass(&config, &mut retries, Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn the_macos_surface_offers_no_trust_row_for_a_paired_device() {
        let device = DeviceInfo {
            address: "AA:BB:CC:DD:EE:FF".into(),
            alias: "Luna 2".into(),
            paired: true,
            trusted: true,
            connected: false,
            services_resolved: false,
            icon: None,
            class: None,
            uuids: Vec::new(),
            rssi: None,
        };

        let payload = devices_payload(
            &[device],
            &[],
            &DiscoveryState::default(),
            None,
            CAPABILITIES,
        );

        let item = &payload["items"][0];
        assert_eq!(item["can_trust"], false);
        assert_eq!(item["can_untrust"], false);
        assert_eq!(item["can_connect"], true);
        assert_eq!(item["can_remove"], true);
    }

    #[test]
    fn a_dual_mode_device_listed_twice_keeps_its_richest_entry() {
        let bare = DeviceInfo {
            address: "AA:BB:CC:DD:EE:FF".into(),
            alias: "Jabra Evolve2 65 Flex".into(),
            paired: true,
            trusted: true,
            connected: false,
            services_resolved: false,
            icon: None,
            class: None,
            uuids: Vec::new(),
            rssi: None,
        };
        let connected = DeviceInfo {
            connected: true,
            uuids: vec!["0000110b-0000-1000-8000-00805f9b34fb".into()],
            ..bare.clone()
        };

        assert!(richer_duplicate(&bare, &connected));
        assert!(!richer_duplicate(&connected, &bare));
    }

    #[test]
    fn addresses_from_iobluetooth_reach_the_domain_in_colon_form() {
        let cases = [
            ("aa-bb-cc-dd-ee-ff", "AA:BB:CC:DD:EE:FF"),
            ("AA:BB:CC:DD:EE:FF", "AA:BB:CC:DD:EE:FF"),
        ];
        for (raw, expected) in cases {
            assert_eq!(colon_separated_address(raw), expected, "{raw}");
            assert_eq!(
                normalize_address(&colon_separated_address(raw)).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn managed_reconnect_keeps_only_configured_devices() {
        let config = test_config(&["aa:bb:cc:dd:ee:ff"]);

        assert!(reconnect_candidates(&config, ReconnectSelection::Managed).is_empty());
    }

    #[test]
    fn unknown_daemon_actions_are_rejected_by_name() {
        match parse_daemon_request(&request("not_a_bluetooth_action")) {
            ReadResult::Error(message) => assert!(message.contains("not_a_bluetooth_action")),
            _ => panic!("unknown actions must be rejected"),
        }
    }

    #[test]
    fn trust_actions_are_refused_with_the_macos_reason() {
        for action in ["trust_device", "untrust_device"] {
            match parse_daemon_request(&request(action)) {
                ReadResult::Error(message) => assert_eq!(message, TRUST_UNSUPPORTED, "{action}"),
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
            match parse_daemon_request(&request(action)) {
                ReadResult::Error(message) => assert!(message.contains("requires an address")),
                _ => panic!("{action} must require an address"),
            }
        }
    }
}
