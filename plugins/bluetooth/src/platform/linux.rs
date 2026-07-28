use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{mpsc, LazyLock, RwLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use bluer::{
    Adapter, AdapterEvent, AdapterProperty, Address, Device, DeviceEvent, DeviceProperty,
    DiscoveryFilter, DiscoveryTransport, ErrorKind, Session, Uuid, UuidExt,
};
use futures::future::pending;
use futures::stream::{LocalBoxStream, SelectAll};
use futures::{Stream, StreamExt};
use qol_headless::DoctorCheckResult;
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_runtime::protocol::{DaemonRequest, DaemonResponse};

use crate::bluetooth::{
    connection_ready, devices_payload, has_audio_class, is_audio_device, managed_device_options,
    normalize_address,
    retry::{RetryPolicy, RetryState},
    search_status_payload, supports_audio_sink, AdapterHealth, DeviceActionState, DeviceInfo,
    DeviceOption, DiscoveryState, ReconnectFailure, ReconnectReport, ReconnectSelection,
};
use crate::config::ReconnectConfig;

const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};
static DISCOVERY_STATE: LazyLock<RwLock<DiscoveryState>> =
    LazyLock::new(|| RwLock::new(DiscoveryState::default()));
static DEVICE_ACTION_STATE: LazyLock<RwLock<Option<DeviceActionState>>> =
    LazyLock::new(|| RwLock::new(None));

type DeviceStreams = SelectAll<LocalBoxStream<'static, (Address, bool)>>;
type DiscoveryStream = LocalBoxStream<'static, AdapterEvent>;
const SEARCH_TIMEOUT: Duration = Duration::from_secs(60);
const PROFILE_CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const DEVICE_CONNECT_ATTEMPTS: u32 = 3;
const DEVICE_CONNECT_RETRY_DELAY: Duration = Duration::from_millis(1500);
const EXPLICIT_DEVICE_ACTION_TIMEOUT: Duration = Duration::from_secs(45);
const BREDR_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);
const AUDIO_SINK_PROFILE: u16 = 0x110b;
const PIPEWIRE_SELF_HEAL_SETTLE: Duration = Duration::from_millis(750);

pub fn required_binaries_check() -> DoctorCheckResult {
    let pactl = executable_on_path("pactl");
    let details = serde_json::json!({
        "platform": "linux",
        "pactl": pactl,
        "executed": false,
    });
    let Some(path) = pactl else {
        return DoctorCheckResult::fail(
            "required_binaries",
            "The pactl audio-profile helper is unavailable on PATH",
        )
        .with_fix("Install PulseAudio utilities or the PipeWire pactl compatibility client")
        .with_details(details);
    };
    DoctorCheckResult::ok(
        "required_binaries",
        format!(
            "pactl executable metadata is available at {}",
            path.display()
        ),
    )
    .with_details(details)
}

fn executable_on_path(program: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|directory| directory.join(program))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

struct DiscoverySession {
    deadline: Instant,
    events: DiscoveryStream,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConnectionMode {
    OneShot,
    Reconnect,
}

impl ConnectionMode {
    fn label(self) -> &'static str {
        match self {
            Self::OneShot => "one_shot",
            Self::Reconnect => "reconnect",
        }
    }
}

impl ExplicitDeviceAction {
    fn label(self) -> &'static str {
        match self {
            Self::Pair => "Pair",
            Self::Connect => "Connect",
        }
    }

    fn trace_name(self) -> &'static str {
        match self {
            Self::Pair => "pair",
            Self::Connect => "connect",
        }
    }
}

#[derive(Clone, Copy)]
enum ExplicitDeviceAction {
    Pair,
    Connect,
}

#[derive(Debug)]
struct DeviceActionTimeout {
    label: &'static str,
    deadline: Duration,
}

impl Display for DeviceActionTimeout {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Bluetooth {} timed out after {} seconds; the device may be unavailable",
            self.label,
            self.deadline.as_secs()
        )
    }
}

impl Error for DeviceActionTimeout {}

#[derive(Clone, Copy)]
enum DaemonCommand {
    Kill,
    SetAdapterPower(bool),
    Pair(Address),
    Trust(Address, bool),
    Connect(Address),
    Disconnect(Address),
    Remove(Address),
    StartSearch,
    StopSearch,
    ReconnectManaged,
    ReconnectTrusted,
    Reload,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DaemonAction {
    Ping,
    Kill,
    EnableAdapter,
    DisableAdapter,
    PairDevice,
    TrustDevice,
    UntrustDevice,
    ConnectDevice,
    DisconnectDevice,
    RemoveDevice,
    StartSearch,
    StopSearch,
    Devices,
    ManagedDeviceOptions,
    SearchStatus,
    AdapterStatus,
    Reconnect,
    ReconnectTrusted,
    Reload,
    Settings,
}

impl TryFrom<&str> for DaemonAction {
    type Error = String;

    fn try_from(action: &str) -> std::result::Result<Self, Self::Error> {
        match action {
            "ping" => Ok(Self::Ping),
            "kill" => Ok(Self::Kill),
            "enable_adapter" => Ok(Self::EnableAdapter),
            "disable_adapter" => Ok(Self::DisableAdapter),
            "pair_device" => Ok(Self::PairDevice),
            "trust_device" => Ok(Self::TrustDevice),
            "untrust_device" => Ok(Self::UntrustDevice),
            "connect_device" => Ok(Self::ConnectDevice),
            "disconnect_device" => Ok(Self::DisconnectDevice),
            "remove_device" => Ok(Self::RemoveDevice),
            "start_search" => Ok(Self::StartSearch),
            "stop_search" => Ok(Self::StopSearch),
            "devices" => Ok(Self::Devices),
            "managed_device_options" => Ok(Self::ManagedDeviceOptions),
            "search_status" => Ok(Self::SearchStatus),
            "adapter_status" => Ok(Self::AdapterStatus),
            "reconnect" => Ok(Self::Reconnect),
            "reconnect_trusted" => Ok(Self::ReconnectTrusted),
            "reload" => Ok(Self::Reload),
            "settings" => Ok(Self::Settings),
            unknown => Err(format!("unknown Bluetooth action: {unknown}")),
        }
    }
}

pub fn list_devices() -> Result<Vec<DeviceInfo>> {
    runtime()?.block_on(async {
        let adapter = default_adapter().await?;
        list_devices_with(&adapter).await
    })
}

pub fn set_adapter_powered(powered: bool) -> Result<AdapterHealth> {
    let result = (|| {
        runtime()?.block_on(async {
            let adapter = default_adapter().await?;
            let mut discovery = None;
            set_adapter_power(&adapter, powered, &mut discovery).await?;
            adapter_health_with(&adapter).await
        })
    })();
    let outcome = if result.is_ok() { "ok" } else { "failed" };
    qol_runtime::probe!(
        "BLUETOOTH_ADAPTER_POWER",
        "source=cli powered={powered} outcome={outcome}"
    );
    result
}

pub fn connect_device(address: &str, power_on_adapter: bool) -> Result<DeviceInfo> {
    let address = parse_address(address)?;
    runtime()?.block_on(async {
        let adapter = default_adapter().await?;
        ensure_powered(&adapter, power_on_adapter).await?;
        let (device, _) = connect_with(&adapter, address, ConnectionMode::OneShot, None).await?;
        Ok(device)
    })
}

pub fn pair_device(address: &str, power_on_adapter: bool) -> Result<DeviceInfo> {
    let address = parse_address(address)?;
    runtime()?.block_on(async {
        let adapter = default_adapter().await?;
        ensure_powered(&adapter, power_on_adapter).await?;
        pair_with(&adapter, address, None).await
    })
}

pub fn set_device_trusted(address: &str, trusted: bool) -> Result<DeviceInfo> {
    let address = parse_address(address)?;
    runtime()?.block_on(async {
        let adapter = default_adapter().await?;
        set_trusted_with(&adapter, address, trusted).await
    })
}

pub fn disconnect_device(address: &str) -> Result<DeviceInfo> {
    let address = parse_address(address)?;
    runtime()?.block_on(async {
        let adapter = default_adapter().await?;
        disconnect_with(&adapter, address).await
    })
}

pub fn remove_device(address: &str) -> Result<()> {
    let address = parse_address(address)?;
    runtime()?.block_on(async {
        let adapter = default_adapter().await?;
        remove_with(&adapter, address).await
    })
}

pub fn reconnect_devices(
    config: &ReconnectConfig,
    selection: ReconnectSelection,
) -> Result<ReconnectReport> {
    runtime()?.block_on(async {
        let adapter = default_adapter().await?;
        ensure_powered(&adapter, config.power_on_adapter).await?;
        reconnect_with(&adapter, config, selection).await
    })
}

pub fn adapter_health() -> Result<AdapterHealth> {
    runtime()?.block_on(async {
        let adapter = default_adapter().await?;
        adapter_health_with(&adapter).await
    })
}

async fn adapter_health_with(adapter: &Adapter) -> Result<AdapterHealth> {
    Ok(AdapterHealth {
        name: adapter.name().to_string(),
        address: adapter.address().await?.to_string(),
        powered: adapter.is_powered().await?,
    })
}

pub fn search_devices(config: &ReconnectConfig) -> Result<Vec<DeviceInfo>> {
    runtime()?.block_on(async {
        let adapter = default_adapter().await?;
        ensure_powered(&adapter, config.power_on_adapter).await?;
        search_until_cancelled(&adapter).await
    })
}

pub fn stop_search() -> Result<()> {
    if core_daemon::send_action(&DAEMON_CONFIG, "stop_search", true) {
        return Ok(());
    }
    bail!("Bluetooth daemon is not reachable")
}

pub fn devices_snapshot() -> Result<serde_json::Value> {
    let devices = list_devices()?;
    let discovery = discovery_state()?;
    let action = DEVICE_ACTION_STATE
        .read()
        .map_err(|_| anyhow!("Bluetooth device action state is unavailable"))?
        .clone();
    let payload = devices_payload(
        &devices,
        &crate::config::load().managed_devices,
        &discovery,
        action.as_ref(),
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
    Ok(adapter_status_payload(&adapter_health()?))
}

fn adapter_status_payload(adapter: &AdapterHealth) -> serde_json::Value {
    serde_json::json!({
        "powered": adapter.powered,
    })
}

fn discovery_state() -> Result<DiscoveryState> {
    DISCOVERY_STATE
        .read()
        .map(|state| state.clone())
        .map_err(|_| anyhow!("Bluetooth discovery state is unavailable"))
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
        Err(error) => Err(format!("Bluetooth daemon query failed: {error}")),
    }
}

pub fn settings_action(action: &str, input: serde_json::Value) -> std::result::Result<(), String> {
    match core_daemon::send_request(&DAEMON_CONFIG, action, input, Duration::from_secs(2)) {
        Ok(DaemonResponse::Handled { .. }) => Ok(()),
        Ok(DaemonResponse::Error { message }) => Err(message),
        Ok(DaemonResponse::Fallback) => Err("Bluetooth daemon declined the action".into()),
        Err(error) => Err(format!("Bluetooth daemon action failed: {error}")),
    }
}

fn current_managed_device_options() -> Result<Vec<DeviceOption>> {
    Ok(managed_device_options(&list_devices()?))
}

pub fn run_daemon(config: ReconnectConfig) -> Result<()> {
    let (listener_tx, listener_rx) = mpsc::channel();
    if !core_daemon::start_request_listener(&DAEMON_CONFIG, listener_tx, parse_daemon_request) {
        bail!("plugin-bluetooth daemon listener failed to start");
    }

    let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::Builder::new()
        .name("bluetooth-command-bridge".into())
        .spawn(move || {
            while let Ok(command) = listener_rx.recv() {
                if command_tx.send(command).is_err() {
                    return;
                }
            }
        })
        .context("failed to start Bluetooth command bridge")?;

    runtime()?.block_on(daemon_loop(config, command_rx))
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("failed to create Bluetooth runtime")
}

async fn default_adapter() -> Result<Adapter> {
    let session = Session::new()
        .await
        .context("failed to connect to the BlueZ system service")?;
    session
        .default_adapter()
        .await
        .context("BlueZ has no default Bluetooth adapter")
}

async fn ensure_powered(adapter: &Adapter, power_on_adapter: bool) -> Result<()> {
    if adapter.is_powered().await? {
        return Ok(());
    }
    if !power_on_adapter {
        bail!("Bluetooth adapter {} is powered off", adapter.name());
    }
    adapter
        .set_powered(true)
        .await
        .with_context(|| format!("failed to power on Bluetooth adapter {}", adapter.name()))
}

async fn list_devices_with(adapter: &Adapter) -> Result<Vec<DeviceInfo>> {
    let mut addresses = adapter.device_addresses().await?;
    addresses.sort();
    let mut devices = Vec::with_capacity(addresses.len());
    for address in addresses {
        devices.push(device_info(&adapter.device(address)?).await?);
    }
    Ok(devices)
}

async fn device_info(device: &Device) -> Result<DeviceInfo> {
    let mut uuids = device
        .uuids()
        .await?
        .unwrap_or_default()
        .into_iter()
        .map(|uuid| uuid.to_string())
        .collect::<Vec<_>>();
    uuids.sort();
    Ok(DeviceInfo {
        address: device.address().to_string(),
        alias: device.alias().await?,
        paired: device.is_paired().await?,
        trusted: device.is_trusted().await?,
        connected: device.is_connected().await?,
        services_resolved: device.is_services_resolved().await?,
        icon: device.icon().await?,
        class: device.class().await?,
        uuids,
        rssi: device.rssi().await?,
    })
}

async fn search_until_cancelled(adapter: &Adapter) -> Result<Vec<DeviceInfo>> {
    let events = adapter
        .discover_devices_with_changes()
        .await
        .context("BlueZ failed to start Bluetooth discovery")?;
    futures::pin_mut!(events);
    let cancelled = tokio::signal::ctrl_c();
    tokio::pin!(cancelled);
    let timeout = tokio::time::sleep(SEARCH_TIMEOUT);
    tokio::pin!(timeout);
    let mut discovery = DiscoveryState::default();
    discovery.start();
    let outcome = loop {
        tokio::select! {
            result = &mut cancelled => {
                result.context("failed to listen for search cancellation")?;
                break "cancelled";
            }
            _ = &mut timeout => break "timed_out",
            event = events.next() => match event {
                Some(AdapterEvent::DeviceAdded(address)) => {
                    if device_is_present(adapter, address).await {
                        discovery.record(address.to_string());
                    }
                }
                Some(AdapterEvent::DeviceRemoved(address)) => discovery.remove(&address.to_string()),
                Some(AdapterEvent::PropertyChanged(_)) => {}
                None => break "ended",
            }
        }
    };

    let mut devices = list_devices_with(adapter).await?;
    devices.retain(|device| device.paired || discovery.contains(&device.address));
    devices.sort_by(|left, right| {
        right
            .connected
            .cmp(&left.connected)
            .then_with(|| right.paired.cmp(&left.paired))
            .then_with(|| right.rssi.cmp(&left.rssi))
            .then_with(|| left.alias.to_lowercase().cmp(&right.alias.to_lowercase()))
            .then_with(|| left.address.cmp(&right.address))
    });
    qol_runtime::probe!(
        "BLUETOOTH_SEARCH",
        "outcome={outcome} devices={}",
        devices.len()
    );
    Ok(devices)
}

async fn device_is_present(adapter: &Adapter, address: Address) -> bool {
    let Ok(device) = adapter.device(address) else {
        return false;
    };
    matches!(device.rssi().await, Ok(Some(_)))
}

async fn connect_with(
    adapter: &Adapter,
    address: Address,
    mode: ConnectionMode,
    cached: Option<DeviceInfo>,
) -> Result<(DeviceInfo, bool)> {
    let mut device = adapter.device(address)?;
    let initial = match device_info(&device).await {
        Ok(info) => info,
        Err(error) => cached.with_context(|| {
            format!("BlueZ dropped {address} before the connection operation began: {error}")
        })?,
    };
    let was_connected = initial.connected;
    if is_audio_device(&initial) && !supports_audio_sink(&initial) {
        if mode == ConnectionMode::Reconnect {
            bail!(
                "{address} is bonded without an A2DP audio profile; use the explicit Connect action to repair it"
            );
        }
        device = prepare_bredr_audio_device(adapter, address, &initial).await?;
    }
    if !device.is_paired().await? {
        device
            .pair()
            .await
            .with_context(|| format!("BlueZ failed to pair {address}"))?;
    }
    if !device.is_trusted().await? {
        device
            .set_trusted(true)
            .await
            .with_context(|| format!("BlueZ failed to trust {address}"))?;
    }
    if !device.is_connected().await? {
        connect_all_profiles(&device, address).await?;
    }
    let paired = device_info(&device).await?;
    if is_audio_device(&paired) {
        if !supports_audio_sink(&paired) {
            bail!("{address} paired without exposing the A2DP audio sink profile");
        }
        ensure_audio_playback_profile(&device, address, mode).await?;
    }
    let info = wait_for_connection_ready(&device, address).await?;
    Ok((info, !was_connected))
}

async fn pair_with(
    adapter: &Adapter,
    address: Address,
    cached: Option<DeviceInfo>,
) -> Result<DeviceInfo> {
    let mut device = adapter.device(address)?;
    let initial = match device_info(&device).await {
        Ok(info) => info,
        Err(error) => cached.with_context(|| {
            format!("BlueZ dropped {address} before the pairing operation began: {error}")
        })?,
    };
    if is_audio_device(&initial) && !supports_audio_sink(&initial) {
        device = prepare_bredr_audio_device(adapter, address, &initial).await?;
    }
    if !device.is_paired().await? {
        device
            .pair()
            .await
            .with_context(|| format!("BlueZ failed to pair {address}"))?;
    }
    let info = device_info(&device).await?;
    if !info.paired {
        bail!("BlueZ returned from Pair but {address} is still unpaired");
    }
    if is_audio_device(&info) && !supports_audio_sink(&info) {
        bail!("{address} paired without exposing the A2DP audio sink profile");
    }
    Ok(info)
}

async fn prepare_bredr_audio_device(
    adapter: &Adapter,
    address: Address,
    current: &DeviceInfo,
) -> Result<Device> {
    if current.connected {
        adapter
            .device(address)?
            .disconnect()
            .await
            .with_context(|| {
                format!("failed to disconnect the incomplete BLE link for {address}")
            })?;
    }
    qol_runtime::probe!(
        "BLUETOOTH_PROFILE_REPAIR",
        "device={} stage=scan_existing transport=bredr paired={} connected={}",
        redacted(address),
        current.paired,
        current.connected,
    );
    discover_bredr_device(adapter, address).await
}

async fn discover_bredr_device(adapter: &Adapter, address: Address) -> Result<Device> {
    adapter
        .set_discovery_filter(DiscoveryFilter {
            transport: DiscoveryTransport::BrEdr,
            pattern: Some(address.to_string()),
            ..Default::default()
        })
        .await
        .context("failed to select BR/EDR Bluetooth discovery")?;
    let result = discover_bredr_device_with_filter(adapter, address).await;
    qol_runtime::probe!(
        "BLUETOOTH_PROFILE_REPAIR",
        "device={} stage=scan_existing outcome={}",
        redacted(address),
        if result.is_ok() { "found" } else { "failed" },
    );
    result
}

async fn discover_bredr_device_with_filter(adapter: &Adapter, address: Address) -> Result<Device> {
    let events = adapter
        .discover_devices_with_changes()
        .await
        .context("BlueZ failed to start BR/EDR discovery")?;
    futures::pin_mut!(events);
    let timeout = tokio::time::sleep(BREDR_DISCOVERY_TIMEOUT);
    tokio::pin!(timeout);
    let mut target_events = 0;
    loop {
        tokio::select! {
            _ = &mut timeout => {
                bail!(
                    "timed out finding {address} over BR/EDR; put the audio device in pairing mode and try Connect again"
                );
            }
            event = events.next() => match event {
                Some(AdapterEvent::DeviceAdded(found)) if found == address => {
                    let device = adapter.device(address)?;
                    target_events += 1;
                    let info = device_info(&device).await?;
                    if supports_audio_sink(&info) || (target_events > 1 && has_audio_class(&info)) {
                        return Ok(device);
                    }
                }
                Some(_) => {}
                None => bail!("BlueZ BR/EDR discovery ended before finding {address}"),
            }
        }
    }
}

async fn connect_all_profiles(device: &Device, address: Address) -> Result<()> {
    let mut attempt = 1;
    loop {
        let result = device.connect().await;
        let outcome = match &result {
            Ok(()) => "connected",
            Err(error) if error.kind == ErrorKind::AlreadyConnected => "already_connected",
            Err(error) if transient_connect_error(&error.kind, &error.message) => "transient",
            Err(_) => "failed",
        };
        qol_runtime::probe!(
            "BLUETOOTH_CONNECT",
            "device={} attempt={attempt} outcome={outcome}",
            redacted(address),
        );
        match result {
            Ok(()) => return Ok(()),
            Err(error) if error.kind == ErrorKind::AlreadyConnected => return Ok(()),
            Err(error)
                if attempt < DEVICE_CONNECT_ATTEMPTS
                    && transient_connect_error(&error.kind, &error.message) =>
            {
                attempt += 1;
                tokio::time::sleep(DEVICE_CONNECT_RETRY_DELAY).await;
            }
            Err(error) => {
                return Err(error).with_context(|| format!("BlueZ failed to connect {address}"))
            }
        }
    }
}

fn transient_connect_error(kind: &ErrorKind, message: &str) -> bool {
    const TRANSIENT_LINK_ERRORS: [&str; 5] = [
        "br-connection-unknown",
        "br-connection-busy",
        "br-connection-canceled",
        "br-connection-aborted-by-remote",
        "br-connection-timeout",
    ];
    *kind == ErrorKind::InProgress || TRANSIENT_LINK_ERRORS.contains(&message)
}

async fn connect_audio_profile(
    device: &Device,
    address: Address,
    mode: ConnectionMode,
) -> Result<()> {
    let profile = Uuid::from_u16(AUDIO_SINK_PROFILE);
    let result = device.connect_profile(&profile).await;
    let outcome = match &result {
        Ok(()) => "connected",
        Err(error) => tolerated_profile_connect(&error.kind).unwrap_or("failed"),
    };
    qol_runtime::probe!(
        "BLUETOOTH_PROFILE_REPAIR",
        "device={} stage=ensure_a2dp mode={} outcome={outcome}",
        redacted(address),
        mode.label(),
    );
    match result {
        Ok(()) => Ok(()),
        Err(error) if tolerated_profile_connect(&error.kind).is_some() => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("BlueZ failed to connect the A2DP profile for {address}")),
    }
}

fn tolerated_profile_connect(kind: &ErrorKind) -> Option<&'static str> {
    match kind {
        ErrorKind::AlreadyConnected => Some("already_connected"),
        ErrorKind::InProgress => Some("in_progress"),
        _ => None,
    }
}

async fn ensure_audio_playback_profile(
    device: &Device,
    address: Address,
    mode: ConnectionMode,
) -> Result<()> {
    connect_audio_profile(device, address, mode).await?;
    if activate_pipewire_a2dp(address, mode).await.is_ok() {
        return Ok(());
    }
    heal_stale_pipewire_transport(device, address, mode).await?;
    let _ = connect_audio_profile(device, address, mode).await;
    activate_pipewire_a2dp(address, mode)
        .await
        .with_context(|| {
            format!("PipeWire A2DP activation for {address} still failed after a self-heal retry")
        })
}

async fn heal_stale_pipewire_transport(
    device: &Device,
    address: Address,
    mode: ConnectionMode,
) -> Result<()> {
    let profile = Uuid::from_u16(AUDIO_SINK_PROFILE);
    device.disconnect_profile(&profile).await.with_context(|| {
        format!("BlueZ failed to disconnect the A2DP profile for {address} during self-heal")
    })?;
    qol_runtime::probe!(
        "BLUETOOTH_PROFILE_REPAIR",
        "device={} stage=self_heal mode={}",
        redacted(address),
        mode.label(),
    );
    tokio::time::sleep(PIPEWIRE_SELF_HEAL_SETTLE).await;
    Ok(())
}

async fn activate_pipewire_a2dp(address: Address, mode: ConnectionMode) -> Result<()> {
    let card = format!("bluez_card.{}", address.to_string().replace(':', "_"));
    let deadline = Instant::now() + PROFILE_CONNECT_TIMEOUT;
    loop {
        let cards = tokio::process::Command::new("pactl")
            .args(["list", "short", "cards"])
            .output()
            .await
            .context("failed to inspect PipeWire Bluetooth cards through pactl")?;
        if cards.status.success() && pactl_has_card(&cards.stdout, &card) {
            let status = tokio::process::Command::new("pactl")
                .args(["set-card-profile", &card, "a2dp-sink"])
                .status()
                .await
                .context("failed to select the PipeWire A2DP profile through pactl")?;
            if status.success() {
                qol_runtime::probe!(
                    "BLUETOOTH_PROFILE_REPAIR",
                    "device={} stage=activate_pipewire_a2dp mode={} outcome=active",
                    redacted(address),
                    mode.label(),
                );
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            qol_runtime::probe!(
                "BLUETOOTH_PROFILE_REPAIR",
                "device={} stage=activate_pipewire_a2dp mode={} outcome=failed",
                redacted(address),
                mode.label(),
            );
            bail!("{address} connected without exposing an activatable PipeWire A2DP profile");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn pactl_has_card(output: &[u8], expected: &str) -> bool {
    String::from_utf8_lossy(output).lines().any(|line| {
        line.split_whitespace()
            .nth(1)
            .is_some_and(|card| card == expected)
    })
}

async fn wait_for_connection_ready(device: &Device, address: Address) -> Result<DeviceInfo> {
    let deadline = Instant::now() + PROFILE_CONNECT_TIMEOUT;
    loop {
        let info = device_info(device).await?;
        if connection_ready(&info) {
            return Ok(info);
        }
        if Instant::now() >= deadline {
            bail!("{address} connected without making its required Bluetooth profiles ready");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn set_trusted_with(
    adapter: &Adapter,
    address: Address,
    trusted: bool,
) -> Result<DeviceInfo> {
    let device = adapter.device(address)?;
    device
        .set_trusted(trusted)
        .await
        .with_context(|| format!("BlueZ failed to set trusted={trusted} for {address}"))?;
    let info = device_info(&device).await?;
    if info.trusted != trusted {
        bail!("BlueZ did not persist trusted={trusted} for {address}");
    }
    Ok(info)
}

async fn remove_with(adapter: &Adapter, address: Address) -> Result<()> {
    adapter
        .remove_device(address)
        .await
        .with_context(|| format!("BlueZ failed to remove {address}"))?;
    remove_discovered_device(address);
    Ok(())
}

async fn disconnect_with(adapter: &Adapter, address: Address) -> Result<DeviceInfo> {
    let device = adapter.device(address)?;
    if device.is_connected().await? {
        device
            .disconnect()
            .await
            .with_context(|| format!("BlueZ failed to disconnect {address}"))?;
    }
    let info = device_info(&device).await?;
    if info.connected {
        bail!("BlueZ returned from Disconnect but {address} is still connected");
    }
    Ok(info)
}

async fn reconnect_with(
    adapter: &Adapter,
    config: &ReconnectConfig,
    selection: ReconnectSelection,
) -> Result<ReconnectReport> {
    let (addresses, mut report) = candidate_addresses(adapter, config, selection).await?;
    for address in addresses {
        let alias = match adapter.device(address) {
            Ok(device) => device.alias().await.unwrap_or_else(|_| address.to_string()),
            Err(_) => address.to_string(),
        };
        match connect_with(adapter, address, ConnectionMode::Reconnect, None).await {
            Ok((device, true)) => report.connected.push(device),
            Ok((device, false)) => report.already_connected.push(device),
            Err(error) => report.failures.push(ReconnectFailure {
                address: address.to_string(),
                alias,
                error: format!("{error:#}"),
            }),
        }
    }
    Ok(report)
}

async fn candidate_addresses(
    adapter: &Adapter,
    config: &ReconnectConfig,
    selection: ReconnectSelection,
) -> Result<(Vec<Address>, ReconnectReport)> {
    if selection == ReconnectSelection::Trusted {
        let devices = list_devices_with(adapter).await?;
        let addresses = devices
            .into_iter()
            .filter(|device| {
                device.paired
                    && device.trusted
                    && (!connection_ready(device) || is_audio_device(device))
            })
            .map(|device| parse_address(&device.address))
            .collect::<Result<Vec<_>>>()?;
        return Ok((addresses, ReconnectReport::default()));
    }

    let mut seen = HashSet::new();
    let mut addresses = Vec::new();
    let mut report = ReconnectReport::default();
    for configured in &config.managed_devices {
        match parse_address(configured) {
            Ok(address) if seen.insert(address) => addresses.push(address),
            Ok(_) => {}
            Err(error) => report.failures.push(ReconnectFailure {
                address: configured.clone(),
                alias: configured.clone(),
                error: error.to_string(),
            }),
        }
    }
    Ok((addresses, report))
}

fn parse_address(value: &str) -> Result<Address> {
    normalize_address(value)?
        .parse()
        .with_context(|| format!("failed to parse Bluetooth address `{value}`"))
}

fn parse_daemon_request(request: &DaemonRequest) -> ReadResult<DaemonCommand> {
    let action = match DaemonAction::try_from(request.action.as_str()) {
        Ok(action) => action,
        Err(error) => return ReadResult::Error(error),
    };
    dispatch_daemon_action(action, request)
}

fn dispatch_daemon_action(
    action: DaemonAction,
    request: &DaemonRequest,
) -> ReadResult<DaemonCommand> {
    match action {
        DaemonAction::Ping => ReadResult::Handled,
        DaemonAction::Kill => ReadResult::Command(DaemonCommand::Kill),
        DaemonAction::EnableAdapter => ReadResult::Command(DaemonCommand::SetAdapterPower(true)),
        DaemonAction::DisableAdapter => ReadResult::Command(DaemonCommand::SetAdapterPower(false)),
        DaemonAction::PairDevice => {
            device_daemon_command(request, DaemonCommand::Pair, "Pairing...")
        }
        DaemonAction::TrustDevice => device_daemon_command(
            request,
            |address| DaemonCommand::Trust(address, true),
            "Trusting...",
        ),
        DaemonAction::UntrustDevice => device_daemon_command(
            request,
            |address| DaemonCommand::Trust(address, false),
            "Removing trust...",
        ),
        DaemonAction::ConnectDevice => {
            device_daemon_command(request, DaemonCommand::Connect, "Connecting...")
        }
        DaemonAction::DisconnectDevice => {
            device_daemon_command(request, DaemonCommand::Disconnect, "Disconnecting...")
        }
        DaemonAction::RemoveDevice => {
            device_daemon_command(request, DaemonCommand::Remove, "Removing...")
        }
        DaemonAction::StartSearch => match mark_search_starting() {
            Ok(()) => ReadResult::Command(DaemonCommand::StartSearch),
            Err(error) => ReadResult::Error(error.to_string()),
        },
        DaemonAction::StopSearch => match mark_search_stopped() {
            Ok(()) => ReadResult::Command(DaemonCommand::StopSearch),
            Err(error) => ReadResult::Error(error.to_string()),
        },
        DaemonAction::Devices => match devices_snapshot() {
            Ok(payload) => ReadResult::HandledWithData(payload),
            Err(error) => ReadResult::Error(format!("{error:#}")),
        },
        DaemonAction::ManagedDeviceOptions => {
            match current_managed_device_options().and_then(|options| {
                serde_json::to_value(options).context("failed to encode Bluetooth device options")
            }) {
                Ok(payload) => ReadResult::HandledWithData(payload),
                Err(error) => ReadResult::Error(format!("{error:#}")),
            }
        }
        DaemonAction::SearchStatus => match search_status_snapshot() {
            Ok(payload) => ReadResult::HandledWithData(payload),
            Err(error) => ReadResult::Error(format!("{error:#}")),
        },
        DaemonAction::AdapterStatus => match adapter_status_snapshot() {
            Ok(payload) => ReadResult::HandledWithData(payload),
            Err(error) => ReadResult::Error(format!("{error:#}")),
        },
        DaemonAction::Reconnect => ReadResult::Command(DaemonCommand::ReconnectManaged),
        DaemonAction::ReconnectTrusted => ReadResult::Command(DaemonCommand::ReconnectTrusted),
        DaemonAction::Reload => ReadResult::Command(DaemonCommand::Reload),
        DaemonAction::Settings => ReadResult::Command(DaemonCommand::Settings),
    }
}

fn device_daemon_command(
    request: &DaemonRequest,
    command: fn(Address) -> DaemonCommand,
    pending_status: &str,
) -> ReadResult<DaemonCommand> {
    let Some(address) = request
        .input
        .get("address")
        .and_then(serde_json::Value::as_str)
    else {
        return ReadResult::Error(format!("{} requires an address", request.action));
    };
    match parse_address(address) {
        Ok(address) => match begin_device_action(address, pending_status) {
            Ok(()) => ReadResult::Command(command(address)),
            Err(error) => ReadResult::Error(error.to_string()),
        },
        Err(error) => ReadResult::Error(error.to_string()),
    }
}

async fn daemon_loop(
    mut config: ReconnectConfig,
    mut commands: tokio::sync::mpsc::UnboundedReceiver<DaemonCommand>,
) -> Result<()> {
    let adapter = default_adapter().await?;
    if config.auto_reconnect {
        if let Err(error) = ensure_powered(&adapter, config.power_on_adapter).await {
            eprintln!("Bluetooth adapter unavailable at daemon start: {error:#}");
        }
    }
    let mut adapter_powered = adapter.is_powered().await?;
    let mut adapter_events = adapter.events().await?.fuse();
    let mut device_streams = DeviceStreams::new();
    let mut discovery: Option<DiscoverySession> = None;
    let mut subscribed = HashSet::new();
    let mut retries = retry_map(&config);
    let known_addresses = adapter.device_addresses().await?;
    subscribe_addresses(
        &adapter,
        &mut device_streams,
        &mut subscribed,
        known_addresses.iter(),
    )
    .await;
    for address in known_addresses {
        spawn_audio_profile_ensure(address);
    }
    if config.auto_reconnect {
        request_all(&mut retries, Instant::now());
    }
    qol_runtime::probe!(
        "BLUETOOTH_START",
        "adapter={} powered={} managed={} auto_reconnect={}",
        adapter.name(),
        adapter_powered,
        retries.len(),
        config.auto_reconnect
    );

    loop {
        let deadline = next_retry_deadline(&retries, adapter_powered);
        let search_deadline = discovery.as_ref().map(|session| session.deadline);
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else {
                    return Ok(());
                };
                if matches!(command, DaemonCommand::Kill) {
                    return Ok(());
                }
                if matches!(command, DaemonCommand::Settings) {
                    if let Err(error) = spawn_settings_panel() {
                        eprintln!("failed to launch native Bluetooth settings: {error:#}");
                        if let Err(error) = crate::settings::open_browser() {
                            eprintln!("failed to open Bluetooth settings fallback: {error:#}");
                        }
                    }
                    continue;
                }
                if let DaemonCommand::SetAdapterPower(powered) = command {
                    match set_adapter_power(&adapter, powered, &mut discovery).await {
                        Ok(()) => {
                            adapter_powered = powered;
                            if powered && config.auto_reconnect {
                                request_all(&mut retries, Instant::now());
                            }
                            qol_runtime::probe!(
                                "BLUETOOTH_ADAPTER_POWER",
                                "source=settings powered={powered} outcome=ok"
                            );
                        }
                        Err(error) => {
                            eprintln!("failed to set Bluetooth adapter power: {error:#}");
                            qol_runtime::probe!(
                                "BLUETOOTH_ADAPTER_POWER",
                                "source=settings powered={powered} outcome=failed"
                            );
                        }
                    }
                    continue;
                }
                if matches!(command, DaemonCommand::StartSearch) {
                    let result = start_search_session(
                        &adapter,
                        config.power_on_adapter,
                        &mut discovery,
                    ).await;
                    if result.is_ok() {
                        adapter_powered = true;
                        continue;
                    }
                    if let Err(error) = result {
                        eprintln!("failed to start Bluetooth search: {error:#}");
                        if let Err(state_error) = mark_search_stopped() {
                            eprintln!("failed to reset Bluetooth search state: {state_error:#}");
                        }
                        qol_runtime::probe!("BLUETOOTH_SEARCH", "outcome=failed stage=start");
                    }
                    continue;
                }
                if matches!(command, DaemonCommand::StopSearch) {
                    if let Err(error) = stop_search_session(&mut discovery, "cancelled") {
                        eprintln!("failed to stop Bluetooth search: {error:#}");
                    }
                    continue;
                }
                if let DaemonCommand::Pair(address) = command {
                    let cached = discovery_state()
                        .ok()
                        .and_then(|state| state.device(&address.to_string()));
                    spawn_explicit_device_action(
                        ExplicitDeviceAction::Pair,
                        address,
                        config.power_on_adapter,
                        cached,
                    );
                    continue;
                }
                if let DaemonCommand::Trust(address, trusted) = command {
                    let (action, failure_label) = if trusted {
                        ("trust", "Trust")
                    } else {
                        ("untrust", "Untrust")
                    };
                    let result = set_trusted_with(&adapter, address, trusted).await;
                    finish_device_action(address, failure_label, &result);
                    trace_device_action(action, address, result);
                    continue;
                }
                if let DaemonCommand::Connect(address) = command {
                    let cached = discovery_state()
                        .ok()
                        .and_then(|state| state.device(&address.to_string()));
                    spawn_explicit_device_action(
                        ExplicitDeviceAction::Connect,
                        address,
                        config.power_on_adapter,
                        cached,
                    );
                    continue;
                }
                if let DaemonCommand::Disconnect(address) = command {
                    let result = disconnect_with(&adapter, address).await;
                    finish_device_action(address, "Disconnect", &result);
                    trace_device_action("disconnect", address, result);
                    continue;
                }
                if let DaemonCommand::Remove(address) = command {
                    let result = remove_with(&adapter, address).await;
                    finish_device_action(address, "Remove", &result);
                    if result.is_ok() {
                        retries.remove(&address);
                        subscribed.remove(&address);
                    }
                    trace_remove_action(address, result);
                    continue;
                }
                if matches!(command, DaemonCommand::Reload) {
                    config = crate::config::load();
                    retries = retry_map(&config);
                    subscribed.retain(|address| retries.contains_key(address));
                    let known_addresses = adapter.device_addresses().await?;
                    subscribe_addresses(
                        &adapter,
                        &mut device_streams,
                        &mut subscribed,
                        known_addresses.iter(),
                    ).await;
                    for address in known_addresses {
                        spawn_audio_profile_ensure(address);
                    }
                    if config.auto_reconnect {
                        request_all(&mut retries, Instant::now());
                    }
                    qol_runtime::probe!(
                        "BLUETOOTH_RELOAD",
                        "managed={} auto_reconnect={}",
                        retries.len(),
                        config.auto_reconnect
                    );
                    continue;
                }
                let selection = if matches!(command, DaemonCommand::ReconnectTrusted) {
                    ReconnectSelection::Trusted
                } else {
                    ReconnectSelection::Managed
                };
                if let Err(error) = ensure_powered(&adapter, config.power_on_adapter).await {
                    trace_manual_failure(&error, selection, "adapter_power");
                    continue;
                }
                adapter_powered = true;
                let report = match reconnect_with(&adapter, &config, selection).await {
                    Ok(report) => report,
                    Err(error) => {
                        trace_manual_failure(&error, selection, "reconnect");
                        continue;
                    }
                };
                apply_report(&mut retries, &report, &config);
                trace_report(&report, selection);
            }
            event = adapter_events.next() => {
                match event {
                    Some(AdapterEvent::DeviceAdded(address)) => {
                        subscribe_one(&adapter, &mut device_streams, &mut subscribed, address).await;
                        if config.auto_reconnect && retries.contains_key(&address) {
                            retries.entry(address).or_default().request_now(Instant::now());
                        }
                        spawn_audio_profile_ensure(address);
                        qol_runtime::probe!("BLUETOOTH_DEVICE_ADDED", "device={}", redacted(address));
                    }
                    Some(AdapterEvent::DeviceRemoved(address)) => {
                        subscribed.remove(&address);
                        if config.auto_reconnect && retries.contains_key(&address) {
                            retries.entry(address).or_default().request_now(Instant::now());
                        }
                        qol_runtime::probe!("BLUETOOTH_DEVICE_REMOVED", "device={}", redacted(address));
                    }
                    Some(AdapterEvent::PropertyChanged(AdapterProperty::Powered(powered))) => {
                        adapter_powered = powered;
                        if powered && config.auto_reconnect {
                            request_all(&mut retries, Instant::now());
                        }
                        if !powered && discovery.is_some() {
                            if let Err(error) = stop_search_session(&mut discovery, "adapter_powered_off") {
                                eprintln!("failed to stop Bluetooth search after adapter power-off: {error:#}");
                            }
                        }
                        qol_runtime::probe!(
                            "BLUETOOTH_ADAPTER_POWER",
                            "source=bluez powered={powered} outcome=observed"
                        );
                    }
                    Some(_) => {}
                    None => bail!("BlueZ adapter event stream ended"),
                }
            }
            event = next_device_event(&mut device_streams) => {
                let (address, connected) = event;
                if connected {
                    spawn_audio_profile_ensure(address);
                }
                if let Some(state) = retries.get_mut(&address) {
                    if connected {
                        state.connected();
                    }
                    if !connected && config.auto_reconnect {
                        state.request_now(Instant::now());
                    }
                }
                qol_runtime::probe!(
                    "BLUETOOTH_CONNECTION",
                    "device={} connected={connected}",
                    redacted(address)
                );
            }
            event = next_discovery_event(&mut discovery) => {
                match event {
                    Some(AdapterEvent::DeviceAdded(address)) => {
                        track_discovered_device(&adapter, address).await;
                    }
                    Some(AdapterEvent::DeviceRemoved(_)) => {}
                    Some(AdapterEvent::PropertyChanged(_)) => {}
                    None => {
                        discovery = None;
                        if let Err(error) = mark_search_stopped() {
                            eprintln!("failed to update Bluetooth search state: {error:#}");
                        }
                        qol_runtime::probe!("BLUETOOTH_SEARCH", "outcome=ended source=bluez");
                    }
                }
            }
            _ = wait_for_deadline(search_deadline) => {
                if let Err(error) = stop_search_session(&mut discovery, "timed_out") {
                    eprintln!("failed to time out Bluetooth search: {error:#}");
                }
            }
            _ = wait_for_deadline(deadline) => {
                attempt_due(&adapter, &config, &mut retries).await;
            }
        }
    }
}

fn spawn_settings_panel() -> Result<()> {
    let executable = std::env::current_exe().context("failed to locate Bluetooth executable")?;
    let mut command = Command::new(executable);
    command.arg(crate::SETTINGS_SURFACE_ARG);
    qol_process::spawn_detached(&mut command).context("failed to launch native Bluetooth settings")
}

fn spawn_explicit_device_action(
    action: ExplicitDeviceAction,
    address: Address,
    power_on_adapter: bool,
    cached: Option<DeviceInfo>,
) {
    std::mem::drop(tokio::spawn(async move {
        let result = complete_device_action_within(
            action.label(),
            EXPLICIT_DEVICE_ACTION_TIMEOUT,
            run_explicit_device_action(action, address, power_on_adapter, cached),
        )
        .await;
        finish_device_action(address, action.label(), &result);
        trace_device_action(action.trace_name(), address, result);
    }));
}

async fn complete_device_action_within<T>(
    label: &'static str,
    deadline: Duration,
    operation: impl Future<Output = Result<T>>,
) -> Result<T> {
    match tokio::time::timeout(deadline, operation).await {
        Ok(result) => result,
        Err(_) => Err(DeviceActionTimeout { label, deadline }.into()),
    }
}

async fn run_explicit_device_action(
    action: ExplicitDeviceAction,
    address: Address,
    power_on_adapter: bool,
    cached: Option<DeviceInfo>,
) -> Result<DeviceInfo> {
    let adapter = default_adapter().await?;
    ensure_powered(&adapter, power_on_adapter).await?;
    match action {
        ExplicitDeviceAction::Pair => pair_with(&adapter, address, cached).await,
        ExplicitDeviceAction::Connect => {
            connect_with(&adapter, address, ConnectionMode::OneShot, cached)
                .await
                .map(|(device, _)| device)
        }
    }
}

fn begin_device_action(address: Address, status: &str) -> Result<()> {
    let mut state = DEVICE_ACTION_STATE
        .write()
        .map_err(|_| anyhow!("Bluetooth device action state is unavailable"))?;
    if state.as_ref().is_some_and(|action| action.pending) {
        bail!("another Bluetooth device action is still running");
    }
    *state = Some(DeviceActionState {
        address: address.to_string(),
        status: status.into(),
        pending: true,
    });
    Ok(())
}

fn finish_device_action<T>(address: Address, label: &str, result: &Result<T>) {
    if let Err(error) = result {
        set_device_action_state(Some(DeviceActionState {
            address: address.to_string(),
            status: format!("{label} failed: {error:#}"),
            pending: false,
        }));
        return;
    }
    set_device_action_state(None);
}

fn set_device_action_state(action: Option<DeviceActionState>) {
    match DEVICE_ACTION_STATE.write() {
        Ok(mut state) => *state = action,
        Err(_) => eprintln!("Bluetooth device action state is unavailable"),
    }
}

fn trace_device_action(action: &str, address: Address, result: Result<DeviceInfo>) {
    match result {
        Ok(device) => qol_runtime::probe!(
            "BLUETOOTH_DEVICE_ACTION",
            "action={action} device={} paired={} trusted={} connected={} services_resolved={} audio={} a2dp_sink={} ready={} outcome=ok",
            redacted(address),
            device.paired,
            device.trusted,
            device.connected,
            device.services_resolved,
            is_audio_device(&device),
            supports_audio_sink(&device),
            connection_ready(&device),
        ),
        Err(error) => {
            eprintln!("Bluetooth {action} failed: {error:#}");
            let outcome = if error.downcast_ref::<DeviceActionTimeout>().is_some() {
                "timed_out"
            } else {
                "failed"
            };
            qol_runtime::probe!(
                "BLUETOOTH_DEVICE_ACTION",
                "action={action} device={} outcome={outcome}",
                redacted(address),
            );
        }
    }
}

fn trace_remove_action(address: Address, result: Result<()>) {
    match result {
        Ok(()) => qol_runtime::probe!(
            "BLUETOOTH_DEVICE_ACTION",
            "action=remove device={} outcome=ok",
            redacted(address),
        ),
        Err(error) => {
            eprintln!("Bluetooth remove failed: {error:#}");
            qol_runtime::probe!(
                "BLUETOOTH_DEVICE_ACTION",
                "action=remove device={} outcome=failed",
                redacted(address),
            );
        }
    }
}

async fn start_search_session(
    adapter: &Adapter,
    power_on_adapter: bool,
    discovery: &mut Option<DiscoverySession>,
) -> Result<()> {
    if discovery.is_some() {
        return Ok(());
    }
    ensure_powered(adapter, power_on_adapter).await?;
    let stream = adapter
        .discover_devices_with_changes()
        .await
        .context("BlueZ failed to start Bluetooth discovery")?
        .boxed_local();
    *discovery = Some(DiscoverySession {
        deadline: Instant::now() + SEARCH_TIMEOUT,
        events: stream,
    });
    qol_runtime::probe!(
        "BLUETOOTH_SEARCH",
        "outcome=started timeout_seconds={}",
        SEARCH_TIMEOUT.as_secs()
    );
    Ok(())
}

async fn set_adapter_power(
    adapter: &Adapter,
    powered: bool,
    discovery: &mut Option<DiscoverySession>,
) -> Result<()> {
    if adapter.is_powered().await? != powered {
        adapter.set_powered(powered).await.with_context(|| {
            format!(
                "failed to power {} Bluetooth adapter {}",
                power_label(powered),
                adapter.name()
            )
        })?;
    }
    if !powered && discovery.is_some() {
        stop_search_session(discovery, "adapter_powered_off")?;
    }
    Ok(())
}

fn power_label(powered: bool) -> &'static str {
    if powered {
        return "on";
    }
    "off"
}

fn mark_search_starting() -> Result<()> {
    let mut state = DISCOVERY_STATE
        .write()
        .map_err(|_| anyhow!("Bluetooth discovery state is unavailable"))?;
    state.start();
    Ok(())
}

fn stop_search_session(
    discovery: &mut Option<DiscoverySession>,
    outcome: &'static str,
) -> Result<()> {
    discovery.take();
    let mut state = DISCOVERY_STATE
        .write()
        .map_err(|_| anyhow!("Bluetooth discovery state is unavailable"))?;
    state.stop();
    qol_runtime::probe!(
        "BLUETOOTH_SEARCH",
        "outcome={outcome} devices={}",
        state.discovered_count()
    );
    Ok(())
}

fn mark_search_stopped() -> Result<()> {
    let mut state = DISCOVERY_STATE
        .write()
        .map_err(|_| anyhow!("Bluetooth discovery state is unavailable"))?;
    state.stop();
    Ok(())
}

async fn track_discovered_device(adapter: &Adapter, address: Address) {
    if !device_is_present(adapter, address).await {
        return;
    }
    let device = match adapter.device(address) {
        Ok(device) => device,
        Err(_) => return,
    };
    let device = match device_info(&device).await {
        Ok(device) => device,
        Err(_) => return,
    };
    let Ok(mut state) = DISCOVERY_STATE.write() else {
        eprintln!("Bluetooth discovery state is unavailable");
        return;
    };
    state.record_device(device);
    qol_runtime::probe!(
        "BLUETOOTH_SEARCH_DEVICE",
        "device={} total={}",
        redacted(address),
        state.discovered_count()
    );
}

fn remove_discovered_device(address: Address) {
    let Ok(mut state) = DISCOVERY_STATE.write() else {
        eprintln!("Bluetooth discovery state is unavailable");
        return;
    };
    state.remove(&address.to_string());
}

async fn next_discovery_event(discovery: &mut Option<DiscoverySession>) -> Option<AdapterEvent> {
    let Some(session) = discovery.as_mut() else {
        return pending().await;
    };
    session.events.next().await
}

fn retry_map(config: &ReconnectConfig) -> HashMap<Address, RetryState> {
    config
        .managed_devices
        .iter()
        .filter_map(|address| parse_address(address).ok())
        .map(|address| (address, RetryState::default()))
        .collect()
}

fn request_all(retries: &mut HashMap<Address, RetryState>, now: Instant) {
    for state in retries.values_mut() {
        state.request_now(now);
    }
}

async fn subscribe_addresses<'a>(
    adapter: &Adapter,
    streams: &mut DeviceStreams,
    subscribed: &mut HashSet<Address>,
    addresses: impl Iterator<Item = &'a Address>,
) {
    let addresses = addresses.copied().collect::<Vec<_>>();
    for address in addresses {
        subscribe_one(adapter, streams, subscribed, address).await;
    }
}

fn spawn_audio_profile_ensure(address: Address) {
    std::mem::drop(tokio::spawn(async move {
        if let Err(error) = ensure_reconnected_audio_profile(address).await {
            eprintln!(
                "Bluetooth A2DP profile restore failed for {}: {error:#}",
                redacted(address)
            );
        }
    }));
}

async fn ensure_reconnected_audio_profile(address: Address) -> Result<()> {
    let adapter = default_adapter().await?;
    let device = adapter.device(address)?;
    let deadline = Instant::now() + PROFILE_CONNECT_TIMEOUT;
    loop {
        let info = device_info(&device).await?;
        if !info.connected || !info.paired || !info.trusted || !is_audio_device(&info) {
            return Ok(());
        }
        if supports_audio_sink(&info) {
            return ensure_audio_playback_profile(&device, address, ConnectionMode::Reconnect)
                .await;
        }
        if Instant::now() >= deadline {
            bail!("{address} reconnected without advertising its A2DP sink profile");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn subscribe_one(
    adapter: &Adapter,
    streams: &mut DeviceStreams,
    subscribed: &mut HashSet<Address>,
    address: Address,
) {
    if subscribed.contains(&address) {
        return;
    }
    let Ok(device) = adapter.device(address) else {
        return;
    };
    let Ok(events) = device.events().await else {
        return;
    };
    let stream = connected_events(events, address).boxed_local();
    streams.push(stream);
    subscribed.insert(address);
}

fn connected_events(
    events: impl Stream<Item = DeviceEvent> + 'static,
    address: Address,
) -> impl Stream<Item = (Address, bool)> {
    events.filter_map(move |event| {
        futures::future::ready(match event {
            DeviceEvent::PropertyChanged(DeviceProperty::Connected(connected)) => {
                Some((address, connected))
            }
            _ => None,
        })
    })
}

async fn next_device_event(streams: &mut DeviceStreams) -> (Address, bool) {
    if streams.is_empty() {
        return pending().await;
    }
    match streams.next().await {
        Some(event) => event,
        None => pending().await,
    }
}

fn next_retry_deadline(
    retries: &HashMap<Address, RetryState>,
    adapter_powered: bool,
) -> Option<Instant> {
    if !adapter_powered {
        return None;
    }
    retries.values().filter_map(RetryState::due).min()
}

async fn wait_for_deadline(deadline: Option<Instant>) {
    let Some(deadline) = deadline else {
        return pending().await;
    };
    tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
}

async fn attempt_due(
    adapter: &Adapter,
    config: &ReconnectConfig,
    retries: &mut HashMap<Address, RetryState>,
) {
    let now = Instant::now();
    let due = retries
        .iter()
        .filter_map(|(address, state)| state.is_due(now).then_some(*address))
        .collect::<Vec<_>>();
    if due.is_empty() {
        return;
    }
    for address in due {
        let attempt = retries
            .get(&address)
            .map(|state| state.failures() + 1)
            .unwrap_or(1);
        qol_runtime::probe!(
            "BLUETOOTH_ATTEMPT",
            "device={} attempt={attempt}",
            redacted(address)
        );
        match connect_with(adapter, address, ConnectionMode::Reconnect, None).await {
            Ok(_) => {
                if let Some(state) = retries.get_mut(&address) {
                    state.connected();
                }
                qol_runtime::probe!(
                    "BLUETOOTH_RESULT",
                    "device={} outcome=connected attempt={attempt}",
                    redacted(address)
                );
            }
            Err(error) => {
                eprintln!(
                    "Bluetooth reconnect failed for {}: {error:#}",
                    redacted(address)
                );
                schedule_failed(retries, &[address], config, Instant::now());
            }
        }
    }
}

fn schedule_failed(
    retries: &mut HashMap<Address, RetryState>,
    addresses: &[Address],
    config: &ReconnectConfig,
    now: Instant,
) {
    let policy = RetryPolicy::from_seconds(config.retry_initial_seconds, config.retry_max_seconds);
    for address in addresses {
        let Some(state) = retries.get_mut(address) else {
            continue;
        };
        let delay = state.failed(now, policy);
        qol_runtime::probe!(
            "BLUETOOTH_RESULT",
            "device={} outcome=retry_scheduled failures={} delay_ms={}",
            redacted(*address),
            state.failures(),
            delay.as_millis()
        );
    }
}

fn apply_report(
    retries: &mut HashMap<Address, RetryState>,
    report: &ReconnectReport,
    config: &ReconnectConfig,
) {
    for device in report.connected.iter().chain(&report.already_connected) {
        let Ok(address) = parse_address(&device.address) else {
            continue;
        };
        if let Some(state) = retries.get_mut(&address) {
            state.connected();
        }
    }
    if !config.auto_reconnect {
        return;
    }
    let failed = report
        .failures
        .iter()
        .filter_map(|failure| parse_address(&failure.address).ok())
        .collect::<Vec<_>>();
    schedule_failed(retries, &failed, config, Instant::now());
}

fn trace_report(report: &ReconnectReport, selection: ReconnectSelection) {
    let selection = selection_label(selection);
    qol_runtime::probe!(
        "BLUETOOTH_MANUAL",
        "selection={selection} connected={} already_connected={} failed={}",
        report.connected.len(),
        report.already_connected.len(),
        report.failures.len()
    );
}

fn trace_manual_failure(error: &anyhow::Error, selection: ReconnectSelection, stage: &str) {
    eprintln!("Bluetooth manual reconnect failed: {error:#}");
    qol_runtime::probe!(
        "BLUETOOTH_MANUAL",
        "selection={} outcome=failed stage={stage}",
        selection_label(selection)
    );
}

fn selection_label(selection: ReconnectSelection) -> &'static str {
    match selection {
        ReconnectSelection::Managed => "managed",
        ReconnectSelection::Trusted => "trusted",
    }
}

fn redacted(address: Address) -> String {
    format!("**:**:**:**:{:02X}:{:02X}", address.0[4], address.0[5])
}

#[cfg(test)]
mod tests {
    use super::{
        begin_device_action, complete_device_action_within, finish_device_action, pactl_has_card,
        parse_address, parse_daemon_request, runtime, set_device_action_state,
        tolerated_profile_connect, transient_connect_error, DaemonAction, DaemonCommand,
        DeviceActionTimeout, Duration, ErrorKind, Instant, ReadResult, Result, RetryState,
        DEVICE_ACTION_STATE,
    };
    use qol_runtime::protocol::DaemonRequest;
    use std::collections::HashMap;

    fn request(action: &str) -> DaemonRequest {
        DaemonRequest {
            action: action.into(),
            input: serde_json::Value::Null,
        }
    }

    #[test]
    fn adapter_power_actions_route_to_the_daemon_loop() {
        let cases = [("enable_adapter", true), ("disable_adapter", false)];
        for (action, expected) in cases {
            assert!(matches!(
                parse_daemon_request(&request(action)),
                ReadResult::Command(DaemonCommand::SetAdapterPower(powered)) if powered == expected
            ));
        }
    }

    #[test]
    fn daemon_actions_parse_at_the_protocol_boundary() {
        let cases = [
            ("ping", DaemonAction::Ping),
            ("kill", DaemonAction::Kill),
            ("enable_adapter", DaemonAction::EnableAdapter),
            ("disable_adapter", DaemonAction::DisableAdapter),
            ("pair_device", DaemonAction::PairDevice),
            ("trust_device", DaemonAction::TrustDevice),
            ("untrust_device", DaemonAction::UntrustDevice),
            ("connect_device", DaemonAction::ConnectDevice),
            ("disconnect_device", DaemonAction::DisconnectDevice),
            ("remove_device", DaemonAction::RemoveDevice),
            ("start_search", DaemonAction::StartSearch),
            ("stop_search", DaemonAction::StopSearch),
            ("devices", DaemonAction::Devices),
            ("managed_device_options", DaemonAction::ManagedDeviceOptions),
            ("search_status", DaemonAction::SearchStatus),
            ("adapter_status", DaemonAction::AdapterStatus),
            ("reconnect", DaemonAction::Reconnect),
            ("reconnect_trusted", DaemonAction::ReconnectTrusted),
            ("reload", DaemonAction::Reload),
            ("settings", DaemonAction::Settings),
        ];
        for (action, expected) in cases {
            assert_eq!(
                DaemonAction::try_from(action),
                Ok(expected),
                "action={action}"
            );
        }
        assert_eq!(
            DaemonAction::try_from("unknown"),
            Err("unknown Bluetooth action: unknown".into())
        );
    }

    #[test]
    fn adapter_status_payload_exposes_the_runtime_power_state() {
        let payload = super::adapter_status_payload(&super::AdapterHealth {
            name: "hci0".into(),
            address: "AA:BB:CC:DD:EE:FF".into(),
            powered: true,
        });
        assert_eq!(
            payload,
            serde_json::json!({
                "powered": true,
            })
        );
    }

    #[test]
    fn powered_off_adapter_suspends_due_reconnects_without_discarding_them() {
        let address = parse_address("AA:BB:CC:DD:EE:FF").unwrap();
        let now = Instant::now();
        let mut retry = RetryState::default();
        retry.request_now(now);
        let retries = HashMap::from([(address, retry)]);

        assert_eq!(super::next_retry_deadline(&retries, false), None);
        assert_eq!(super::next_retry_deadline(&retries, true), Some(now));
    }

    #[test]
    fn explicit_device_action_deadline_clears_the_pending_state_with_an_error() {
        let address = parse_address("AA:BB:CC:DD:EE:FF").unwrap();
        begin_device_action(address, "Connecting...").unwrap();
        let result = runtime().unwrap().block_on(complete_device_action_within(
            "Connect",
            Duration::ZERO,
            futures::future::pending::<Result<()>>(),
        ));
        finish_device_action(address, "Connect", &result);
        let state = DEVICE_ACTION_STATE.read().unwrap().clone().unwrap();
        let error = result.unwrap_err();

        assert_eq!(
            error.to_string(),
            "Bluetooth Connect timed out after 0 seconds; the device may be unavailable"
        );
        assert!(error.downcast_ref::<DeviceActionTimeout>().is_some());
        assert_eq!(state.address, address.to_string());
        assert_eq!(
            state.status,
            "Connect failed: Bluetooth Connect timed out after 0 seconds; the device may be unavailable"
        );
        assert!(!state.pending);
        set_device_action_state(None);
    }

    #[test]
    fn transient_link_errors_are_retried() {
        let cases = [
            (ErrorKind::Failed, "br-connection-unknown", true),
            (ErrorKind::Failed, "br-connection-busy", true),
            (ErrorKind::Failed, "br-connection-canceled", true),
            (ErrorKind::Failed, "br-connection-aborted-by-remote", true),
            (ErrorKind::Failed, "br-connection-timeout", true),
            (ErrorKind::InProgress, "br-connection-busy", true),
            (ErrorKind::Failed, "br-connection-page-timeout", false),
            (
                ErrorKind::Failed,
                "br-connection-adapter-not-powered",
                false,
            ),
            (ErrorKind::Failed, "", false),
            (ErrorKind::NotReady, "resource not ready", false),
        ];
        for (kind, message, expected) in cases {
            assert_eq!(
                transient_connect_error(&kind, message),
                expected,
                "kind: {kind:?} message: {message}"
            );
        }
    }

    #[test]
    fn transient_profile_connect_outcomes_are_tolerated() {
        let cases = [
            (ErrorKind::AlreadyConnected, Some("already_connected")),
            (ErrorKind::InProgress, Some("in_progress")),
            (ErrorKind::Failed, None),
            (ErrorKind::ConnectionAttemptFailed, None),
            (ErrorKind::NotReady, None),
        ];
        for (kind, expected) in cases {
            assert_eq!(tolerated_profile_connect(&kind), expected, "kind: {kind:?}");
        }
    }

    #[test]
    fn finds_an_exact_pipewire_bluetooth_card() {
        let output =
            b"43\talsa_card.pci\talsa\n555\tbluez_card.74_68_59_7F_5F_E9\tmodule-bluez5-device.c\n";
        assert!(pactl_has_card(output, "bluez_card.74_68_59_7F_5F_E9"));
        assert!(!pactl_has_card(output, "bluez_card.88_0E_85_16_CA_67"));
    }
}
