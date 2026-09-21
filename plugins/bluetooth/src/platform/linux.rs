use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::future::Future;
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
use qol_audio::attempts::record::{read_ownership, OwnershipState};
use qol_audio::attempts::{AudioWatchState, RepairDecision};
use qol_audio::control::{self, Card, Sink, Source, SourceOutput};
use qol_audio::devices::State;
use qol_headless::DoctorCheckResult;
use qol_host_fixes::{findings_payload, HostFixes};
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_plugin_daemon::notification::send_notification;
use qol_runtime::protocol::{DaemonRequest, DaemonResponse};

use crate::bluetooth::{
    adapter_options, audio_output_degraded, audio_profile_repairable, connection_ready,
    devices_payload, has_audio_class, is_audio_device, managed_device_options, normalize_address,
    retry::{RetryPolicy, RetryState},
    search_status_payload, supports_audio_sink, AdapterHealth, AdapterInfo, BackendCapabilities,
    DeviceActionState, DeviceInfo, DeviceOption, DiscoveryState, ReconnectFailure, ReconnectReport,
    ReconnectSelection,
};
use crate::config::ReconnectConfig;
use crate::hostfix::BluetoothHostFixes;
use crate::platform::audio;

pub const CAPABILITIES: BackendCapabilities = BackendCapabilities {
    separate_trust_flag: true,
    audio_reclaim: crate::audio_claim::platform::RECLAIM_SUPPORTED,
};

const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};
static DISCOVERY_STATE: LazyLock<RwLock<DiscoveryState>> =
    LazyLock::new(|| RwLock::new(DiscoveryState::default()));
static DEVICE_ACTION_STATE: LazyLock<RwLock<Option<DeviceActionState>>> =
    LazyLock::new(|| RwLock::new(None));
static ADAPTER_STATE: LazyLock<RwLock<Option<AdapterHealth>>> = LazyLock::new(|| RwLock::new(None));
static AUDIO_REPAIRS: LazyLock<RwLock<HashSet<Address>>> =
    LazyLock::new(|| RwLock::new(HashSet::new()));
static ADOPTION_RETRIES: LazyLock<RwLock<AdoptionRetries>> =
    LazyLock::new(|| RwLock::new(AdoptionRetries::default()));
static CONNECTION_FLIGHTS: LazyLock<RwLock<HashSet<Address>>> =
    LazyLock::new(|| RwLock::new(HashSet::new()));

type DeviceStreams = SelectAll<LocalBoxStream<'static, (Address, bool)>>;
type DiscoveryStream = LocalBoxStream<'static, AdapterEvent>;
const SEARCH_TIMEOUT: Duration = Duration::from_secs(60);
const PROFILE_CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const AUDIO_PROFILE_WATCH_INTERVAL: Duration = Duration::from_secs(10);
const AUDIO_REPAIR_COOLDOWN: Duration = Duration::from_secs(60);
const AUDIO_REPAIR_MAX_ATTEMPTS: u32 = 3;
const AUDIO_WATCH_TICK_BUDGET: Duration = Duration::from_secs(3);
const ADOPTION_RETRY_WINDOW: Duration = Duration::from_secs(600);
const ADOPT_TRIGGER_RETRY: &str = "retry";
const DISCONNECT_SETTLE_TIMEOUT: Duration = Duration::from_secs(5);
const DEVICE_CONNECT_ATTEMPTS: u32 = 3;
const DEVICE_CONNECT_RETRY_DELAY: Duration = Duration::from_millis(1500);
const EXPLICIT_DEVICE_ACTION_TIMEOUT: Duration = Duration::from_secs(45);
const BREDR_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(15);
const AUDIO_SINK_PROFILE: u16 = 0x110b;
const PIPEWIRE_SELF_HEAL_SETTLE: Duration = Duration::from_millis(750);
const ADAPTER_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const ADAPTER_RETRY_DELAY: Duration = Duration::from_secs(5);

pub fn required_binaries_check() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        "required_binaries",
        "Bluetooth audio-profile control uses the bundled native Pulse client.",
    )
    .with_details(serde_json::json!({
        "platform": "linux",
        "bundled_client": true,
        "executed": false,
    }))
}

struct DiscoverySession {
    deadline: Instant,
    events: DiscoveryStream,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AudioAdoption {
    Adopt,
    Keep,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PipewireActivation {
    Active,
    Stale,
    ServerUnavailable,
}

impl PipewireActivation {
    fn label(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Stale => "stale",
            Self::ServerUnavailable => "server_unavailable",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AudioProfile {
    Active,
    Absent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConnectionMode {
    OneShot,
    Reconnect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConnectionSource {
    Explicit,
    ManualReconnect,
    AutoRetry,
    AudioRepair,
}

impl ConnectionSource {
    fn label(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::ManualReconnect => "manual_reconnect",
            Self::AutoRetry => "auto_retry",
            Self::AudioRepair => "audio_repair",
        }
    }
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
    ReclaimDevice,
    RemoveDevice,
    StartSearch,
    StopSearch,
    Devices,
    ManagedDeviceOptions,
    AdapterOptions,
    SearchStatus,
    AdapterStatus,
    Reconnect,
    ReconnectTrusted,
    Reload,
    Settings,
    HostFixes,
    ApplyHostFix,
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
            "reclaim_device" => Ok(Self::ReclaimDevice),
            "remove_device" => Ok(Self::RemoveDevice),
            "start_search" => Ok(Self::StartSearch),
            "stop_search" => Ok(Self::StopSearch),
            "devices" => Ok(Self::Devices),
            "managed_device_options" => Ok(Self::ManagedDeviceOptions),
            "adapter_options" => Ok(Self::AdapterOptions),
            "search_status" => Ok(Self::SearchStatus),
            "adapter_status" => Ok(Self::AdapterStatus),
            "reconnect" => Ok(Self::Reconnect),
            "reconnect_trusted" => Ok(Self::ReconnectTrusted),
            "reload" => Ok(Self::Reload),
            "settings" => Ok(Self::Settings),
            "host_fixes" => Ok(Self::HostFixes),
            "apply_host_fix" => Ok(Self::ApplyHostFix),
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
        let (device, _) = connect_with(
            &adapter,
            address,
            ConnectionMode::OneShot,
            ConnectionSource::Explicit,
            None,
        )
        .await?;
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
    let devices = if adapter_available()? {
        list_devices()?
    } else {
        Vec::new()
    };
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
    let adapter = ADAPTER_STATE
        .read()
        .map_err(|_| anyhow!("Bluetooth adapter state is unavailable"))?
        .clone();
    Ok(adapter_status_payload(adapter.as_ref()))
}

fn adapter_status_payload(adapter: Option<&AdapterHealth>) -> serde_json::Value {
    serde_json::json!({
        "available": adapter.is_some(),
        "powered": adapter.is_some_and(|adapter| adapter.powered),
    })
}

fn adapter_available() -> Result<bool> {
    ADAPTER_STATE
        .read()
        .map(|adapter| adapter.is_some())
        .map_err(|_| anyhow!("Bluetooth adapter state is unavailable"))
}

fn set_adapter_state(adapter: Option<AdapterHealth>) {
    match ADAPTER_STATE.write() {
        Ok(mut state) => *state = adapter,
        Err(_) => eprintln!("Bluetooth adapter state is unavailable"),
    }
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

fn current_managed_device_options() -> Result<Vec<DeviceOption>> {
    if !adapter_available()? {
        return Ok(Vec::new());
    }
    Ok(managed_device_options(&list_devices()?))
}

fn current_adapter_options() -> Result<Vec<DeviceOption>> {
    Ok(adapter_options(&runtime()?.block_on(adapter_inventory())?))
}

pub fn run_daemon(config: ReconnectConfig) -> Result<()> {
    crate::hostfix::restore_claimed_managers();
    notify_adopted_managers();
    let (listener_tx, listener_rx) = mpsc::channel();
    if !core_daemon::start_request_listener(&DAEMON_CONFIG, listener_tx, parse_daemon_request) {
        bail!("plugin-bluetooth daemon listener failed to start");
    }

    let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();
    let signal_tx = command_tx.clone();
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

    let outcome = runtime()?.block_on(async move {
        tokio::spawn(async move {
            let Ok(mut terminate) =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            else {
                return;
            };
            let _ = terminate.recv().await;
            let _ = signal_tx.send(DaemonCommand::Kill);
        });
        resilient_daemon_loop(config, command_rx).await
    });
    crate::hostfix::restore_claimed_managers();
    outcome
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
    let configured = crate::config::load().adapter;
    if configured.is_empty() {
        return session
            .default_adapter()
            .await
            .context("BlueZ has no default Bluetooth adapter");
    }
    let target = normalize_address(&configured)?;
    for name in session.adapter_names().await? {
        let Ok(adapter) = session.adapter(&name) else {
            continue;
        };
        let Ok(address) = adapter.address().await else {
            continue;
        };
        if address.to_string().to_ascii_uppercase() == target {
            return Ok(adapter);
        }
    }
    bail!("configured Bluetooth adapter {target} was not found; set the adapter back to Automatic or reconnect it")
}

async fn adapter_inventory() -> Result<Vec<AdapterInfo>> {
    let session = Session::new()
        .await
        .context("failed to connect to the BlueZ system service")?;
    let mut adapters = Vec::new();
    for name in session.adapter_names().await? {
        let Ok(adapter) = session.adapter(&name) else {
            continue;
        };
        let Ok(address) = adapter.address().await else {
            continue;
        };
        let mut paired_count = 0;
        for device_address in adapter.device_addresses().await.unwrap_or_default() {
            let Ok(device) = adapter.device(device_address) else {
                continue;
            };
            if device.is_paired().await.unwrap_or(false) {
                paired_count += 1;
            }
        }
        adapters.push(AdapterInfo {
            name: name.clone(),
            address: address.to_string().to_ascii_uppercase(),
            paired_count,
        });
    }
    Ok(adapters)
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
                    discovery.record(address.to_string());
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

async fn connect_with(
    adapter: &Adapter,
    address: Address,
    mode: ConnectionMode,
    source: ConnectionSource,
    cached: Option<DeviceInfo>,
) -> Result<(DeviceInfo, bool)> {
    let Some(_flight) = ConnectionFlightGuard::acquire(address) else {
        qol_runtime::probe!(
            "BLUETOOTH_CONNECT",
            "device={} source={} outcome=already_in_flight",
            redacted(address),
            source.label(),
        );
        bail!(
            "Bluetooth connection for {} is already in flight",
            redacted(address)
        );
    };
    let mut device = adapter.device(address)?;
    let initial = match device_info(&device).await {
        Ok(info) => info,
        Err(error) => cached.with_context(|| {
            format!(
                "BlueZ dropped {} before the connection operation began: {error}",
                redacted(address)
            )
        })?,
    };
    let was_connected = initial.connected;
    if is_audio_device(&initial) && !supports_audio_sink(&initial) {
        if mode == ConnectionMode::Reconnect {
            bail!(
                "{} is bonded without an A2DP audio profile; use the explicit Connect action to repair it",
                redacted(address)
            );
        }
        device = prepare_bredr_audio_device(adapter, address, &initial).await?;
    }
    if !device.is_paired().await? {
        device
            .pair()
            .await
            .with_context(|| format!("BlueZ failed to pair {}", redacted(address)))?;
    }
    if !device.is_trusted().await? {
        device
            .set_trusted(true)
            .await
            .with_context(|| format!("BlueZ failed to trust {}", redacted(address)))?;
    }
    if !device.is_connected().await? {
        connect_all_profiles(&device, address, source).await?;
    }
    let paired = device_info(&device).await?;
    if is_audio_device(&paired) {
        if !supports_audio_sink(&paired) {
            bail!(
                "{} paired without exposing the A2DP audio sink profile",
                redacted(address)
            );
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
            format!(
                "BlueZ dropped {} before the pairing operation began: {error}",
                redacted(address)
            )
        })?,
    };
    if is_audio_device(&initial) && !supports_audio_sink(&initial) {
        device = prepare_bredr_audio_device(adapter, address, &initial).await?;
    }
    if !device.is_paired().await? {
        device
            .pair()
            .await
            .with_context(|| format!("BlueZ failed to pair {}", redacted(address)))?;
    }
    let info = device_info(&device).await?;
    if !info.paired {
        bail!(
            "BlueZ returned from Pair but {} is still unpaired",
            redacted(address)
        );
    }
    if is_audio_device(&info) && !supports_audio_sink(&info) {
        bail!(
            "{} paired without exposing the A2DP audio sink profile",
            redacted(address)
        );
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
                format!(
                    "failed to disconnect the incomplete BLE link for {}",
                    redacted(address)
                )
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
                    "timed out finding {} over BR/EDR; put the audio device in pairing mode and try Connect again",
                    redacted(address)
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
                None => bail!(
                    "BlueZ BR/EDR discovery ended before finding {}",
                    redacted(address)
                ),
            }
        }
    }
}

async fn connect_all_profiles(
    device: &Device,
    address: Address,
    source: ConnectionSource,
) -> Result<()> {
    let mut attempt = 1;
    loop {
        let result = device.connect().await;
        match &result {
            Ok(()) => qol_runtime::probe!(
                "BLUETOOTH_CONNECT",
                "device={} source={} attempt={attempt} outcome=connected",
                redacted(address),
                source.label(),
            ),
            Err(error) => qol_runtime::probe!(
                "BLUETOOTH_CONNECT",
                "device={} source={} attempt={attempt} outcome={} kind={:?} reason={}",
                redacted(address),
                source.label(),
                connect_error_outcome(&error.kind, &error.message),
                error.kind,
                error.message,
            ),
        }
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
                return Err(error)
                    .with_context(|| format!("BlueZ failed to connect {}", redacted(address)))
            }
        }
    }
}

fn transient_connect_error(kind: &ErrorKind, message: &str) -> bool {
    const TRANSIENT_LINK_ERRORS: [&str; 6] = [
        "br-connection-unknown",
        "br-connection-busy",
        "br-connection-canceled",
        "br-connection-aborted-by-remote",
        "br-connection-timeout",
        "br-connection-create-socket",
    ];
    *kind == ErrorKind::InProgress || TRANSIENT_LINK_ERRORS.contains(&message)
}

fn connect_error_outcome(kind: &ErrorKind, message: &str) -> &'static str {
    if *kind == ErrorKind::AlreadyConnected {
        return "already_connected";
    }
    if transient_connect_error(kind, message) {
        return "transient";
    }
    "failed"
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
        "device={} source=profile_repair stage=ensure_a2dp mode={} outcome={outcome}",
        redacted(address),
        mode.label(),
    );
    match result {
        Ok(()) => Ok(()),
        Err(error) if tolerated_profile_connect(&error.kind).is_some() => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "BlueZ failed to connect the A2DP profile for {}",
                redacted(address)
            )
        }),
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
    match activate_pipewire_a2dp(address, mode).await? {
        PipewireActivation::Active => return Ok(()),
        PipewireActivation::ServerUnavailable => {
            bail!("the audio server is unreachable, so the Bluetooth link for {} was left connected without routing", redacted(address))
        }
        PipewireActivation::Stale => {}
    }
    heal_stale_pipewire_transport(device, address, mode).await?;
    let _ = connect_audio_profile(device, address, mode).await;
    match activate_pipewire_a2dp(address, mode).await? {
        PipewireActivation::Active => Ok(()),
        PipewireActivation::Stale | PipewireActivation::ServerUnavailable => {
            bail!(
                "PipeWire A2DP activation for {} still failed after a self-heal retry",
                redacted(address)
            )
        }
    }
}

async fn heal_stale_pipewire_transport(
    device: &Device,
    address: Address,
    mode: ConnectionMode,
) -> Result<()> {
    let profile = Uuid::from_u16(AUDIO_SINK_PROFILE);
    device.disconnect_profile(&profile).await.with_context(|| {
        format!(
            "BlueZ failed to disconnect the A2DP profile for {} during self-heal",
            redacted(address)
        )
    })?;
    qol_runtime::probe!(
        "BLUETOOTH_PROFILE_REPAIR",
        "device={} source=profile_repair stage=self_heal mode={}",
        redacted(address),
        mode.label(),
    );
    tokio::time::sleep(PIPEWIRE_SELF_HEAL_SETTLE).await;
    Ok(())
}

async fn activate_pipewire_a2dp(
    address: Address,
    mode: ConnectionMode,
) -> Result<PipewireActivation> {
    let card = audio::card_name(&address.to_string());
    let deadline = Instant::now() + PROFILE_CONNECT_TIMEOUT;
    loop {
        let cards = audio::request("inspect audio cards", control::list_cards).await;
        let card_present = cards
            .as_ref()
            .is_ok_and(|cards| card_for(cards, &card).is_some());
        if card_present && select_a2dp_sink(&card).await {
            qol_runtime::probe!(
                "BLUETOOTH_PROFILE_REPAIR",
                "device={} source=profile_repair stage=activate_pipewire_a2dp mode={} outcome=active",
                redacted(address),
                mode.label(),
            );
            return Ok(PipewireActivation::Active);
        }
        if Instant::now() >= deadline {
            let outcome = if cards.is_ok() {
                PipewireActivation::Stale
            } else {
                PipewireActivation::ServerUnavailable
            };
            qol_runtime::probe!(
                "BLUETOOTH_PROFILE_REPAIR",
                "device={} source=profile_repair stage=activate_pipewire_a2dp mode={} outcome={}",
                redacted(address),
                mode.label(),
                outcome.label(),
            );
            return Ok(outcome);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn card_for<'a>(cards: &'a [Card], name: &str) -> Option<&'a Card> {
    cards.iter().find(|card| card.name == name)
}

fn active_card_profile(cards: &[Card], name: &str) -> Option<String> {
    card_for(cards, name)?.active_profile.clone()
}

async fn select_a2dp_sink(card: &str) -> bool {
    let card = card.to_owned();
    let request = move || control::set_card_profile(&card, "a2dp-sink");
    let selected = audio::request("select the A2DP audio profile", request).await;
    selected.is_ok()
}

async fn live_audio_profile(address: Address) -> Option<String> {
    let card = audio::card_name(&address.to_string());
    let cards = audio::request("inspect audio cards", control::list_cards).await;
    let cards = cards.ok()?;
    active_card_profile(&cards, &card)
}

async fn microphone_in_use(address: Address) -> Result<bool> {
    let prefix = audio::source_prefix(&address.to_string());
    let sources = audio::request("inspect audio inputs", control::list_sources).await?;
    let Some(source_index) = source_index_matching(&sources, &prefix) else {
        return Ok(false);
    };
    let outputs = audio::request("inspect input streams", control::list_source_outputs).await?;
    Ok(source_in_use(&outputs, source_index))
}

fn source_index_matching(sources: &[Source], prefix: &str) -> Option<u32> {
    sources
        .iter()
        .find(|source| source.name.starts_with(prefix))
        .map(|source| source.index)
}

fn source_in_use(outputs: &[SourceOutput], source_index: u32) -> bool {
    outputs
        .iter()
        .any(|output| output.source_index == source_index)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AdoptionOutcome {
    Adopted,
    Blocked(String),
    Unverified,
    SinkAbsent,
}

fn adopt_sink_wait(trigger: &str) -> Duration {
    if trigger == ADOPT_TRIGGER_RETRY {
        Duration::ZERO
    } else {
        PROFILE_CONNECT_TIMEOUT
    }
}

async fn adopt_default_sink(address: Address, trigger: &'static str) -> Result<AdoptionOutcome> {
    let prefix = audio::sink_prefix(&address.to_string());
    let deadline = Instant::now() + adopt_sink_wait(trigger);
    loop {
        let listing = audio::request("inspect audio outputs", control::list_sinks).await;
        let listing_unavailable = match listing {
            Ok(sinks) => {
                if let Some(blocking) = running_sink_except(&sinks, &prefix) {
                    return Ok(AdoptionOutcome::Blocked(blocking));
                }
                if let Some(sink) = audio::sink_matching(&sinks, &prefix) {
                    let sink = sink.name.clone();
                    let request = move || control::set_default_sink(&sink);
                    let selected = audio::request("select the default output", request).await;
                    if selected.is_ok() {
                        qol_runtime::probe!(
                            "BLUETOOTH_DEFAULT_OUTPUT",
                            "device={} outcome=adopted trigger={trigger}",
                            redacted(address)
                        );
                        return Ok(AdoptionOutcome::Adopted);
                    }
                }
                false
            }
            Err(_) => true,
        };
        if Instant::now() >= deadline {
            if listing_unavailable {
                return Ok(AdoptionOutcome::Unverified);
            }
            return Ok(AdoptionOutcome::SinkAbsent);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn running_sink_except(sinks: &[Sink], prefix: &str) -> Option<String> {
    sinks
        .iter()
        .find(|sink| sink.state == State::Running && !sink.name.starts_with(prefix))
        .map(|sink| sink.name.clone())
}

async fn other_output_playing(address: Address) -> Result<Option<String>> {
    let prefix = audio::sink_prefix(&address.to_string());
    let sinks = audio::request("inspect audio outputs", control::list_sinks)
        .await
        .context("failed to inspect audio outputs through the bundled audio client")?;
    Ok(running_sink_except(&sinks, &prefix))
}

async fn wait_for_connection_ready(device: &Device, address: Address) -> Result<DeviceInfo> {
    let deadline = Instant::now() + PROFILE_CONNECT_TIMEOUT;
    loop {
        let info = device_info(device).await?;
        if connection_ready(&info) {
            return Ok(info);
        }
        if Instant::now() >= deadline {
            bail!(
                "{} connected without making its required Bluetooth profiles ready",
                redacted(address)
            );
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
    device.set_trusted(trusted).await.with_context(|| {
        format!(
            "BlueZ failed to set trusted={trusted} for {}",
            redacted(address)
        )
    })?;
    let info = device_info(&device).await?;
    if info.trusted != trusted {
        bail!(
            "BlueZ did not persist trusted={trusted} for {}",
            redacted(address)
        );
    }
    Ok(info)
}

async fn remove_with(adapter: &Adapter, address: Address) -> Result<()> {
    adapter
        .remove_device(address)
        .await
        .with_context(|| format!("BlueZ failed to remove {}", redacted(address)))?;
    remove_discovered_device(address);
    Ok(())
}

async fn disconnect_with(adapter: &Adapter, address: Address) -> Result<DeviceInfo> {
    let device = adapter.device(address)?;
    if device.is_connected().await? {
        device
            .disconnect()
            .await
            .with_context(|| format!("BlueZ failed to disconnect {}", redacted(address)))?;
    }
    let info = device_info(&device).await?;
    if info.connected {
        bail!(
            "BlueZ returned from Disconnect but {} is still connected",
            redacted(address)
        );
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
        match connect_with(
            adapter,
            address,
            ConnectionMode::Reconnect,
            ConnectionSource::ManualReconnect,
            None,
        )
        .await
        {
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
        DaemonAction::PairDevice => device_daemon_command(request, DaemonCommand::Pair, "Pairing"),
        DaemonAction::TrustDevice => device_daemon_command(
            request,
            |address| DaemonCommand::Trust(address, true),
            "Trusting",
        ),
        DaemonAction::UntrustDevice => device_daemon_command(
            request,
            |address| DaemonCommand::Trust(address, false),
            "Removing trust",
        ),
        DaemonAction::ConnectDevice => {
            device_daemon_command(request, DaemonCommand::Connect, "Connecting")
        }
        DaemonAction::DisconnectDevice => {
            device_daemon_command(request, DaemonCommand::Disconnect, "Disconnecting")
        }
        DaemonAction::ReclaimDevice => reclaim_daemon_result(request),
        DaemonAction::RemoveDevice => {
            device_daemon_command(request, DaemonCommand::Remove, "Removing")
        }
        DaemonAction::StartSearch => ReadResult::Command(DaemonCommand::StartSearch),
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
        DaemonAction::AdapterOptions => {
            match current_adapter_options().and_then(|options| {
                serde_json::to_value(options).context("failed to encode Bluetooth adapter options")
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
        DaemonAction::HostFixes => {
            ReadResult::HandledWithData(findings_payload(&BluetoothHostFixes.detect()))
        }
        DaemonAction::ApplyHostFix => match host_fix_id(request) {
            Ok(id) => {
                spawn_host_fix(id);
                ReadResult::Handled
            }
            Err(message) => ReadResult::Error(message),
        },
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

fn notify_adopted_managers() {
    for message in crate::hostfix::adopt_competing_managers_if_resident() {
        send_notification("Bluetooth", &message);
    }
}

fn device_daemon_command(
    request: &DaemonRequest,
    command: fn(Address) -> DaemonCommand,
    pending_status: &str,
) -> ReadResult<DaemonCommand> {
    match request_address(request) {
        Ok(address) => match begin_device_action(address, pending_status) {
            Ok(()) => ReadResult::Command(command(address)),
            Err(error) => ReadResult::Error(error.to_string()),
        },
        Err(error) => ReadResult::Error(error),
    }
}

fn request_address(request: &DaemonRequest) -> std::result::Result<Address, String> {
    let Some(address) = request
        .input
        .get("address")
        .and_then(serde_json::Value::as_str)
    else {
        return Err(format!("{} requires an address", request.action));
    };
    parse_address(address).map_err(|error| error.to_string())
}

fn reclaim_daemon_result(request: &DaemonRequest) -> ReadResult<DaemonCommand> {
    match request_address(request) {
        Ok(address) => match crate::audio_claim::platform::reclaim_output(&address.to_string()) {
            Ok(()) => ReadResult::Handled,
            Err(error) => ReadResult::Error(format!("{error:#}")),
        },
        Err(error) => ReadResult::Error(error),
    }
}

async fn resilient_daemon_loop(
    mut config: ReconnectConfig,
    mut commands: tokio::sync::mpsc::UnboundedReceiver<DaemonCommand>,
) -> Result<()> {
    loop {
        let adapter = match tokio::time::timeout(ADAPTER_CONNECT_TIMEOUT, default_adapter()).await {
            Ok(Ok(adapter)) => adapter,
            Ok(Err(error)) => {
                set_adapter_state(None);
                reset_discovery_state("adapter_unavailable")?;
                eprintln!("Bluetooth adapter unavailable; retrying: {error:#}");
                qol_runtime::probe!("BLUETOOTH_ADAPTER", "available=false outcome=retrying");
                if wait_without_adapter(&mut config, &mut commands).await? {
                    return Ok(());
                }
                continue;
            }
            Err(_) => {
                set_adapter_state(None);
                reset_discovery_state("adapter_lookup_timed_out")?;
                eprintln!(
                    "Bluetooth adapter lookup timed out after {} seconds; retrying",
                    ADAPTER_CONNECT_TIMEOUT.as_secs()
                );
                qol_runtime::probe!("BLUETOOTH_ADAPTER", "available=false outcome=timed_out");
                if wait_without_adapter(&mut config, &mut commands).await? {
                    return Ok(());
                }
                continue;
            }
        };
        match daemon_loop(&mut config, &mut commands, adapter).await {
            Ok(()) => return Ok(()),
            Err(error) => {
                set_adapter_state(None);
                reset_discovery_state("adapter_session_ended")?;
                eprintln!("Bluetooth adapter session ended; retrying: {error:#}");
                qol_runtime::probe!("BLUETOOTH_ADAPTER", "available=false outcome=session_ended");
            }
        }
        if wait_without_adapter(&mut config, &mut commands).await? {
            return Ok(());
        }
    }
}

async fn wait_without_adapter(
    config: &mut ReconnectConfig,
    commands: &mut tokio::sync::mpsc::UnboundedReceiver<DaemonCommand>,
) -> Result<bool> {
    let delay = tokio::time::sleep(ADAPTER_RETRY_DELAY);
    tokio::pin!(delay);
    loop {
        notify_adopted_managers();
        tokio::select! {
            _ = &mut delay => return Ok(false),
            command = commands.recv() => {
                let Some(command) = command else {
                    return Ok(true);
                };
                match command {
                    DaemonCommand::Kill => return Ok(true),
                    DaemonCommand::Settings => {
                        if let Err(error) = spawn_settings_panel() {
                            eprintln!("failed to launch native Bluetooth settings: {error:#}");
                            if let Err(error) = crate::settings::open_browser() {
                                eprintln!("failed to open Bluetooth settings fallback: {error:#}");
                            }
                        }
                    }
                    DaemonCommand::Reload => {
                        *config = crate::config::load();
                        qol_runtime::probe!(
                            "BLUETOOTH_RELOAD",
                            "managed={} auto_reconnect={} adapter_available=false",
                            config.managed_devices.len(),
                            config.auto_reconnect
                        );
                    }
                    DaemonCommand::StopSearch => {
                        mark_search_stopped()?;
                    }
                    DaemonCommand::StartSearch => {
                        reset_discovery_state("adapter_unavailable")?;
                        qol_runtime::probe!(
                            "BLUETOOTH_SEARCH",
                            "outcome=failed stage=adapter_unavailable"
                        );
                    }
                    command => fail_command_without_adapter(command),
                }
            }
        }
    }
}

fn fail_command_without_adapter(command: DaemonCommand) {
    let Some((address, label, action)) = unavailable_device_command(command) else {
        match command {
            DaemonCommand::SetAdapterPower(_)
            | DaemonCommand::ReconnectManaged
            | DaemonCommand::ReconnectTrusted => {
                qol_runtime::probe!(
                    "BLUETOOTH_ACTION",
                    "outcome=failed reason=adapter_unavailable"
                );
            }
            _ => {}
        }
        return;
    };
    let result: Result<()> = Err(anyhow!("Bluetooth adapter is unavailable"));
    finish_device_action(address, label, &result);
    qol_runtime::probe!(
        "BLUETOOTH_DEVICE_ACTION",
        "action={action} device={} outcome=failed reason=adapter_unavailable",
        redacted(address)
    );
}

fn unavailable_device_command(
    command: DaemonCommand,
) -> Option<(Address, &'static str, &'static str)> {
    match command {
        DaemonCommand::Pair(address) => Some((address, "Pair", "pair")),
        DaemonCommand::Trust(address, true) => Some((address, "Trust", "trust")),
        DaemonCommand::Trust(address, false) => Some((address, "Untrust", "untrust")),
        DaemonCommand::Connect(address) => Some((address, "Connect", "connect")),
        DaemonCommand::Disconnect(address) => Some((address, "Disconnect", "disconnect")),
        DaemonCommand::Remove(address) => Some((address, "Remove", "remove")),
        DaemonCommand::SetAdapterPower(_)
        | DaemonCommand::ReconnectManaged
        | DaemonCommand::ReconnectTrusted
        | DaemonCommand::Kill
        | DaemonCommand::StartSearch
        | DaemonCommand::StopSearch
        | DaemonCommand::Reload
        | DaemonCommand::Settings => None,
    }
}

async fn daemon_loop(
    config: &mut ReconnectConfig,
    commands: &mut tokio::sync::mpsc::UnboundedReceiver<DaemonCommand>,
    adapter: Adapter,
) -> Result<()> {
    if config.auto_reconnect && !config.managed_devices.is_empty() {
        if let Err(error) = ensure_powered(&adapter, config.power_on_adapter).await {
            eprintln!("Bluetooth adapter unavailable at daemon start: {error:#}");
        }
    }
    let mut adapter_powered = adapter.is_powered().await?;
    set_adapter_state(Some(adapter_health_with(&adapter).await?));
    let mut adapter_events = adapter.events().await?.fuse();
    let mut device_streams = DeviceStreams::new();
    let mut discovery: Option<DiscoverySession> = None;
    let mut subscribed = HashSet::new();
    let mut retries = retry_map(config);
    let mut watch_states: HashMap<Address, AudioWatchState> = HashMap::new();
    let mut manager_reconcile = tokio::time::interval(Duration::from_secs(5));
    manager_reconcile.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut audio_watch = tokio::time::interval(AUDIO_PROFILE_WATCH_INTERVAL);
    audio_watch.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let known_addresses = adapter.device_addresses().await?;
    subscribe_addresses(
        &adapter,
        &mut device_streams,
        &mut subscribed,
        known_addresses.iter(),
    )
    .await;
    for address in known_addresses {
        spawn_audio_profile_ensure(address, AudioAdoption::Keep);
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
                        watch_states.remove(&address);
                    }
                    trace_remove_action(address, result);
                    continue;
                }
                if matches!(command, DaemonCommand::Reload) {
                    *config = crate::config::load();
                    retries = retry_map(config);
                    let known_addresses = adapter.device_addresses().await?;
                    subscribe_addresses(
                        &adapter,
                        &mut device_streams,
                        &mut subscribed,
                        known_addresses.iter(),
                    ).await;
                    for address in known_addresses {
                        spawn_audio_profile_ensure(address, AudioAdoption::Keep);
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
                let report = match reconnect_with(&adapter, config, selection).await {
                    Ok(report) => report,
                    Err(error) => {
                        trace_manual_failure(&error, selection, "reconnect");
                        continue;
                    }
                };
                apply_report(&mut retries, &report, config);
                trace_report(&report, selection);
            }
            event = adapter_events.next() => {
                match event {
                    Some(AdapterEvent::DeviceAdded(address)) => {
                        subscribe_one(&adapter, &mut device_streams, &mut subscribed, address).await;
                        if discovery.is_some() {
                            track_discovered_device(&adapter, address).await;
                        }
                        if config.auto_reconnect && retries.contains_key(&address) {
                            retries.entry(address).or_default().request_when_idle(Instant::now());
                        }
                        spawn_audio_profile_ensure(address, AudioAdoption::Adopt);
                        qol_runtime::probe!(
                            "BLUETOOTH_DEVICE_ADDED",
                            "device={} search_active={}",
                            redacted(address),
                            discovery.is_some()
                        );
                    }
                    Some(AdapterEvent::DeviceRemoved(address)) => {
                        subscribed.remove(&address);
                        if config.auto_reconnect && retries.contains_key(&address) {
                            retries.entry(address).or_default().request_when_idle(Instant::now());
                        }
                        qol_runtime::probe!("BLUETOOTH_DEVICE_REMOVED", "device={}", redacted(address));
                    }
                    Some(AdapterEvent::PropertyChanged(AdapterProperty::Powered(powered))) => {
                        adapter_powered = powered;
                        set_adapter_state(Some(AdapterHealth {
                            name: adapter.name().to_string(),
                            address: adapter.address().await?.to_string(),
                            powered,
                        }));
                        if powered && config.auto_reconnect {
                            request_all(&mut retries, Instant::now());
                        }
                        if !powered {
                            discovery.take();
                            if let Err(error) = reset_discovery_state("adapter_powered_off") {
                                eprintln!("failed to reset Bluetooth search after adapter power-off: {error:#}");
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
                let qol_in_flight = ConnectionFlightGuard::active(address);
                if connected && !qol_in_flight {
                    spawn_audio_profile_ensure(address, AudioAdoption::Adopt);
                }
                if !connected {
                    clear_adoption_retry(address);
                }
                if !connected && !qol_in_flight {
                    watch_states.remove(&address);
                }
                if let Some(state) = retries.get_mut(&address) {
                    if connected {
                        state.connected();
                    }
                    if !connected && config.auto_reconnect {
                        state.request_when_idle(Instant::now());
                    }
                }
                qol_runtime::probe!(
                    "BLUETOOTH_CONNECTION",
                    "device={} connected={connected} origin={}",
                    redacted(address),
                    if qol_in_flight {
                        "qol_in_flight"
                    } else {
                        "device_observed"
                    }
                );
            }
            event = next_discovery_event(&mut discovery) => {
                match event {
                    Some(AdapterEvent::DeviceAdded(address)) => {
                        track_discovered_device(&adapter, address).await;
                    }
                    Some(AdapterEvent::DeviceRemoved(address)) => {
                        remove_discovered_device(address);
                    }
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
                attempt_due(&adapter, config, &mut retries).await;
            }
            _ = manager_reconcile.tick() => {
                notify_adopted_managers();
                crate::hostfix::reconcile_claimed_managers();
            }
            _ = audio_watch.tick() => {
                audio_watch_tick(&adapter, &mut watch_states).await;
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
        ExplicitDeviceAction::Connect => connect_with(
            &adapter,
            address,
            ConnectionMode::OneShot,
            ConnectionSource::Explicit,
            cached,
        )
        .await
        .map(|(device, _)| device),
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
        .discover_devices()
        .await
        .context("BlueZ failed to start Bluetooth discovery")?
        .boxed_local();
    mark_search_starting()?;
    *discovery = Some(DiscoverySession {
        deadline: Instant::now() + SEARCH_TIMEOUT,
        events: stream,
    });
    for address in adapter.device_addresses().await? {
        track_discovered_device(adapter, address).await;
    }
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
    if !powered {
        discovery.take();
        reset_discovery_state("adapter_powered_off")?;
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

fn reset_discovery_state(reason: &'static str) -> Result<()> {
    let mut state = DISCOVERY_STATE
        .write()
        .map_err(|_| anyhow!("Bluetooth discovery state is unavailable"))?;
    let searching = state.searching();
    let devices = state.discovered_count();
    state.reset();
    if searching || devices > 0 {
        qol_runtime::probe!(
            "BLUETOOTH_SEARCH",
            "outcome=reset reason={reason} devices={devices}"
        );
    }
    Ok(())
}

async fn track_discovered_device(adapter: &Adapter, address: Address) {
    let total = {
        let Ok(mut state) = DISCOVERY_STATE.write() else {
            eprintln!("Bluetooth discovery state is unavailable");
            return;
        };
        state.record(address.to_string());
        state.discovered_count()
    };
    qol_runtime::probe!(
        "BLUETOOTH_SEARCH_DEVICE",
        "device={} total={total} outcome=recorded",
        redacted(address)
    );

    let device = match adapter.device(address) {
        Ok(device) => device,
        Err(error) => {
            eprintln!(
                "failed to resolve discovered Bluetooth device {}: {error:#}",
                redacted(address)
            );
            qol_runtime::probe!(
                "BLUETOOTH_SEARCH_DEVICE",
                "device={} outcome=partial stage=resolve",
                redacted(address)
            );
            return;
        }
    };
    let device = match device_info(&device).await {
        Ok(device) => device,
        Err(error) => {
            eprintln!(
                "failed to inspect discovered Bluetooth device {}: {error:#}",
                redacted(address)
            );
            qol_runtime::probe!(
                "BLUETOOTH_SEARCH_DEVICE",
                "device={} outcome=partial stage=inspect",
                redacted(address)
            );
            return;
        }
    };
    let Ok(mut state) = DISCOVERY_STATE.write() else {
        eprintln!("Bluetooth discovery state is unavailable");
        return;
    };
    state.record_device(device);
    qol_runtime::probe!(
        "BLUETOOTH_SEARCH_DEVICE",
        "device={} total={} outcome=complete",
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

struct ConnectionFlightGuard(Address);

impl ConnectionFlightGuard {
    fn acquire(address: Address) -> Option<Self> {
        let claimed = {
            let mut active = CONNECTION_FLIGHTS.write().ok()?;
            active.insert(address)
        };
        claimed.then(|| Self(address))
    }

    fn active(address: Address) -> bool {
        CONNECTION_FLIGHTS
            .read()
            .map(|active| active.contains(&address))
            .unwrap_or(true)
    }
}

impl Drop for ConnectionFlightGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = CONNECTION_FLIGHTS.write() {
            active.remove(&self.0);
        }
    }
}

struct AudioRepairGuard(Address);

impl AudioRepairGuard {
    fn acquire(address: Address) -> Option<Self> {
        let claimed = {
            let mut active = AUDIO_REPAIRS.write().ok()?;
            active.insert(address)
        };
        claimed.then(|| Self(address))
    }
}

impl Drop for AudioRepairGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = AUDIO_REPAIRS.write() {
            active.remove(&self.0);
        }
    }
}

fn spawn_audio_profile_ensure(address: Address, adoption: AudioAdoption) {
    std::mem::drop(tokio::spawn(async move {
        let Some(_flight) = ConnectionFlightGuard::acquire(address) else {
            qol_runtime::probe!(
                "BLUETOOTH_PROFILE_REPAIR",
                "device={} stage=ensure_a2dp source=device_reconnect outcome=already_in_flight",
                redacted(address)
            );
            return;
        };
        let Some(_guard) = AudioRepairGuard::acquire(address) else {
            qol_runtime::probe!(
                "BLUETOOTH_PROFILE_REPAIR",
                "device={} stage=ensure_a2dp outcome=already_in_flight",
                redacted(address)
            );
            return;
        };
        match ensure_reconnected_audio_profile(address).await {
            Ok(AudioProfile::Active) => {
                adopt_reconnected_output(address, adoption, "connect").await
            }
            Ok(AudioProfile::Absent) => {}
            Err(error) => eprintln!(
                "Bluetooth A2DP profile restore failed for {}: {error:#}",
                redacted(address)
            ),
        }
    }));
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AdoptionDecision {
    Adopt,
    SkipBlocked(String),
    SkipUnverified,
    SkipDisabled,
    SkipKeep,
    SkipOwned(String),
}

fn adoption_decision(
    adoption: AudioAdoption,
    enabled: bool,
    ownership: Result<OwnershipState, qol_audio::AudioError>,
    guard: Result<Option<String>>,
) -> AdoptionDecision {
    if adoption == AudioAdoption::Keep {
        return AdoptionDecision::SkipKeep;
    }
    if !enabled {
        return AdoptionDecision::SkipDisabled;
    }
    match ownership {
        Ok(OwnershipState::None) => {}
        Ok(OwnershipState::Held(ownership)) => {
            return AdoptionDecision::SkipOwned(ownership.owner);
        }
        Ok(OwnershipState::Unreadable(_)) | Err(_) => {
            return AdoptionDecision::SkipUnverified;
        }
    }
    match guard {
        Ok(Some(blocking)) => AdoptionDecision::SkipBlocked(blocking),
        Ok(None) => AdoptionDecision::Adopt,
        Err(_) => AdoptionDecision::SkipUnverified,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WatchAction {
    Repair,
    Retry,
    Idle,
}

fn watch_action(degraded: bool, retry_pending: bool) -> WatchAction {
    if degraded {
        return WatchAction::Repair;
    }
    if retry_pending {
        return WatchAction::Retry;
    }
    WatchAction::Idle
}

#[derive(Debug, Default)]
struct AdoptionRetries {
    pending: HashMap<Address, Instant>,
}

impl AdoptionRetries {
    fn register(&mut self, address: Address, now: Instant) -> bool {
        let known = self.pending.contains_key(&address);
        self.pending.entry(address).or_insert(now);
        !known
    }

    fn clear(&mut self, address: Address) {
        self.pending.remove(&address);
    }

    fn is_pending(&self, address: Address) -> bool {
        self.pending.contains_key(&address)
    }

    fn expire(&mut self, now: Instant) -> Vec<Address> {
        let mut expired = Vec::new();
        self.pending.retain(|address, first| {
            let live = now.saturating_duration_since(*first) < ADOPTION_RETRY_WINDOW;
            if !live {
                expired.push(*address);
            }
            live
        });
        expired
    }
}

fn register_adoption_retry(address: Address) -> bool {
    ADOPTION_RETRIES
        .write()
        .map(|mut retries| retries.register(address, Instant::now()))
        .unwrap_or(false)
}

fn clear_adoption_retry(address: Address) {
    if let Ok(mut retries) = ADOPTION_RETRIES.write() {
        retries.clear(address);
    }
}

fn adoption_retry_pending(address: Address) -> bool {
    ADOPTION_RETRIES
        .read()
        .is_ok_and(|retries| retries.is_pending(address))
}

fn expire_adoption_retries() -> Vec<Address> {
    ADOPTION_RETRIES
        .write()
        .map(|mut retries| retries.expire(Instant::now()))
        .unwrap_or_default()
}

fn redacted_sink(name: &str) -> String {
    let Some(rest) = name.strip_prefix("bluez_output.") else {
        return name.to_string();
    };
    let Some((mac, instance)) = rest.split_once('.') else {
        return name.to_string();
    };
    let octets: Vec<&str> = mac.split('_').collect();
    let valid = octets.len() == 6
        && octets
            .iter()
            .all(|octet| octet.len() == 2 && octet.bytes().all(|byte| byte.is_ascii_hexdigit()));
    if !valid {
        return name.to_string();
    }
    format!(
        "bluez_output.**:**:**:**:{}:{}.{}",
        octets[4], octets[5], instance
    )
}

fn record_adoption_skip_blocked(address: Address, blocking: &str) {
    if !register_adoption_retry(address) {
        return;
    }
    let blocking = redacted_sink(blocking);
    eprintln!(
        "Bluetooth default output selection skipped for {}: {blocking} is playing",
        redacted(address)
    );
    qol_runtime::probe!(
        "BLUETOOTH_DEFAULT_OUTPUT",
        "device={} outcome=skipped reason=other_output_playing other_sink={}",
        redacted(address),
        qol_runtime::probe::token(&blocking)
    );
}

fn record_adoption_skip_unverified(address: Address) {
    if !register_adoption_retry(address) {
        return;
    }
    eprintln!(
        "Bluetooth default output selection skipped for {}: the PipeWire sink listing is unavailable",
        redacted(address)
    );
    qol_runtime::probe!(
        "BLUETOOTH_DEFAULT_OUTPUT",
        "device={} outcome=skipped reason=guard_unavailable",
        redacted(address)
    );
}

fn record_adoption_skip_owned(address: Address, owner: &str) {
    if !register_adoption_retry(address) {
        return;
    }
    eprintln!(
        "Bluetooth default output selection skipped for {}: {owner} holds the machine default",
        redacted(address)
    );
    qol_runtime::probe!(
        "BLUETOOTH_DEFAULT_OUTPUT",
        "device={} outcome=skipped reason=output_owned owner={}",
        redacted(address),
        qol_runtime::probe::token(owner)
    );
}

fn record_adoption_sink_absent(address: Address) {
    if !register_adoption_retry(address) {
        return;
    }
    eprintln!(
        "Bluetooth default output selection deferred for {}: the device has no PipeWire sink yet",
        redacted(address)
    );
    qol_runtime::probe!(
        "BLUETOOTH_DEFAULT_OUTPUT",
        "device={} outcome=skipped reason=sink_absent",
        redacted(address)
    );
}

async fn adopt_reconnected_output(
    address: Address,
    adoption: AudioAdoption,
    trigger: &'static str,
) {
    let decision = adoption_decision(
        adoption,
        crate::config::load().set_default_output,
        read_ownership(),
        other_output_playing(address).await,
    );
    match decision {
        AdoptionDecision::Adopt => match adopt_default_sink(address, trigger).await {
            Ok(AdoptionOutcome::Adopted) => clear_adoption_retry(address),
            Ok(AdoptionOutcome::Blocked(blocking)) => {
                record_adoption_skip_blocked(address, &blocking)
            }
            Ok(AdoptionOutcome::Unverified) => record_adoption_skip_unverified(address),
            Ok(AdoptionOutcome::SinkAbsent) => record_adoption_sink_absent(address),
            Err(error) => eprintln!(
                "Bluetooth default output selection failed for {}: {error:#}",
                redacted(address)
            ),
        },
        AdoptionDecision::SkipBlocked(blocking) => record_adoption_skip_blocked(address, &blocking),
        AdoptionDecision::SkipUnverified => record_adoption_skip_unverified(address),
        AdoptionDecision::SkipOwned(owner) => record_adoption_skip_owned(address, &owner),
        AdoptionDecision::SkipDisabled | AdoptionDecision::SkipKeep => {
            clear_adoption_retry(address)
        }
    }
}

async fn audio_watch_tick(adapter: &Adapter, states: &mut HashMap<Address, AudioWatchState>) {
    for address in expire_adoption_retries() {
        eprintln!(
            "Bluetooth default output selection abandoned for {}: the retry window expired",
            redacted(address)
        );
        qol_runtime::probe!(
            "BLUETOOTH_DEFAULT_OUTPUT",
            "device={} outcome=abandoned reason=window_expired",
            redacted(address)
        );
    }
    let scan = async {
        for address in adapter.device_addresses().await.unwrap_or_default() {
            let Ok(device) = adapter.device(address) else {
                continue;
            };
            let Ok(info) = device_info(&device).await else {
                continue;
            };
            if !info.connected
                || !info.paired
                || !info.trusted
                || !is_audio_device(&info)
                || !supports_audio_sink(&info)
            {
                continue;
            }
            let profile = live_audio_profile(address).await;
            match watch_action(
                audio_output_degraded(profile.as_deref()),
                adoption_retry_pending(address),
            ) {
                WatchAction::Retry => {
                    adopt_reconnected_output(address, AudioAdoption::Adopt, ADOPT_TRIGGER_RETRY)
                        .await;
                    continue;
                }
                WatchAction::Idle => continue,
                WatchAction::Repair => {}
            }
            qol_runtime::probe!(
                "BLUETOOTH_PROFILE_REPAIR",
                "device={} stage=watch outcome=degraded profile={}",
                redacted(address),
                profile.as_deref().unwrap_or("none"),
            );
            if ConnectionFlightGuard::active(address) {
                qol_runtime::probe!(
                    "BLUETOOTH_PROFILE_REPAIR",
                    "device={} stage=watch outcome=in_flight",
                    redacted(address)
                );
                continue;
            }
            match microphone_in_use(address).await {
                Ok(true) => {
                    qol_runtime::probe!(
                        "BLUETOOTH_PROFILE_REPAIR",
                        "device={} stage=watch outcome=mic_in_use",
                        redacted(address)
                    );
                    continue;
                }
                Ok(false) => {}
                Err(error) => {
                    eprintln!(
                        "Bluetooth A2DP repair deferred for {}: the microphone state could not be verified: {error:#}",
                        redacted(address)
                    );
                    qol_runtime::probe!(
                        "BLUETOOTH_PROFILE_REPAIR",
                        "device={} stage=watch outcome=mic_unverified",
                        redacted(address)
                    );
                    continue;
                }
            }
            let now = Instant::now();
            let state = states.entry(address).or_default();
            match state.decide(now, AUDIO_REPAIR_COOLDOWN, AUDIO_REPAIR_MAX_ATTEMPTS) {
                RepairDecision::Repair => {
                    state.attempted(now);
                    let attempts = state.attempts();
                    qol_runtime::probe!(
                        "BLUETOOTH_PROFILE_REPAIR",
                        "device={} stage=watch outcome=spawned attempt={attempts} profile={}",
                        redacted(address),
                        profile.as_deref().unwrap_or("none"),
                    );
                    spawn_audio_profile_reconnect(address);
                }
                RepairDecision::Cooldown => {
                    qol_runtime::probe!(
                        "BLUETOOTH_PROFILE_REPAIR",
                        "device={} stage=watch outcome=cooldown",
                        redacted(address)
                    );
                }
                RepairDecision::Exhausted => {
                    qol_runtime::probe!(
                        "BLUETOOTH_PROFILE_REPAIR",
                        "device={} stage=watch outcome=exhausted",
                        redacted(address)
                    );
                }
            }
        }
    };
    let _ = tokio::time::timeout(AUDIO_WATCH_TICK_BUDGET, scan).await;
}

fn spawn_audio_profile_reconnect(address: Address) {
    std::mem::drop(tokio::spawn(async move {
        let Some(_flight) = ConnectionFlightGuard::acquire(address) else {
            qol_runtime::probe!(
                "BLUETOOTH_PROFILE_REPAIR",
                "device={} stage=reconnect outcome=already_in_flight",
                redacted(address)
            );
            return;
        };
        let Some(_repair) = AudioRepairGuard::acquire(address) else {
            qol_runtime::probe!(
                "BLUETOOTH_PROFILE_REPAIR",
                "device={} stage=reconnect outcome=already_in_flight",
                redacted(address)
            );
            return;
        };
        let repair = tokio::time::timeout(
            EXPLICIT_DEVICE_ACTION_TIMEOUT,
            reconnect_audio_profile(address),
        )
        .await;
        match repair {
            Ok(Ok(AudioProfile::Active)) => {
                adopt_reconnected_output(address, AudioAdoption::Adopt, "repair").await;
            }
            Ok(Ok(AudioProfile::Absent)) => {}
            Ok(Err(error)) => eprintln!(
                "Bluetooth A2DP reconnect repair failed for {}: {error:#}",
                redacted(address)
            ),
            Err(_) => eprintln!(
                "Bluetooth A2DP reconnect repair failed for {}: the repair exceeded {} seconds",
                redacted(address),
                EXPLICIT_DEVICE_ACTION_TIMEOUT.as_secs()
            ),
        }
    }));
}

async fn reconnect_audio_profile(address: Address) -> Result<AudioProfile> {
    let adapter = default_adapter().await?;
    let device = adapter.device(address)?;
    let info = device_info(&device).await?;
    if !info.connected || !info.paired || !info.trusted || !is_audio_device(&info) {
        return Ok(AudioProfile::Absent);
    }
    qol_runtime::probe!(
        "BLUETOOTH_PROFILE_REPAIR",
        "device={} stage=reconnect outcome=disconnecting",
        redacted(address)
    );
    device.disconnect().await.with_context(|| {
        format!(
            "failed to disconnect {} for the A2DP repair",
            redacted(address)
        )
    })?;
    let deadline = Instant::now() + DISCONNECT_SETTLE_TIMEOUT;
    loop {
        if !device.is_connected().await? {
            break;
        }
        if Instant::now() >= deadline {
            bail!(
                "{} did not disconnect before the A2DP repair deadline",
                redacted(address)
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    connect_all_profiles(&device, address, ConnectionSource::AudioRepair).await?;
    ensure_audio_playback_profile(&device, address, ConnectionMode::Reconnect).await?;
    let profile = live_audio_profile(address).await;
    if audio_output_degraded(profile.as_deref()) {
        let profile = profile.unwrap_or_else(|| "none".to_string());
        qol_runtime::probe!(
            "BLUETOOTH_PROFILE_REPAIR",
            "device={} stage=reconnect outcome=failed profile={profile}",
            redacted(address)
        );
        bail!(
            "{} reconnected but PipeWire stayed on {profile}",
            redacted(address)
        );
    }
    qol_runtime::probe!(
        "BLUETOOTH_PROFILE_REPAIR",
        "device={} stage=reconnect outcome=connected profile={}",
        redacted(address),
        profile.as_deref().unwrap_or("none"),
    );
    Ok(AudioProfile::Active)
}

async fn ensure_reconnected_audio_profile(address: Address) -> Result<AudioProfile> {
    let adapter = default_adapter().await?;
    let device = adapter.device(address)?;
    let deadline = Instant::now() + PROFILE_CONNECT_TIMEOUT;
    loop {
        let info = device_info(&device).await?;
        if !info.connected || !info.paired || !info.trusted || !is_audio_device(&info) {
            return Ok(AudioProfile::Absent);
        }
        if audio_profile_repairable(&info) {
            ensure_audio_playback_profile(&device, address, ConnectionMode::Reconnect).await?;
            return Ok(AudioProfile::Active);
        }
        if Instant::now() >= deadline {
            bail!(
                "{} reconnected without resolving its A2DP sink profile",
                redacted(address)
            );
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
            "device={} source={} attempt={attempt}",
            redacted(address),
            ConnectionSource::AutoRetry.label()
        );
        match connect_with(
            adapter,
            address,
            ConnectionMode::Reconnect,
            ConnectionSource::AutoRetry,
            None,
        )
        .await
        {
            Ok(_) => {
                if let Some(state) = retries.get_mut(&address) {
                    state.connected();
                }
                qol_runtime::probe!(
                    "BLUETOOTH_RESULT",
                    "device={} source={} outcome=connected attempt={attempt}",
                    redacted(address),
                    ConnectionSource::AutoRetry.label()
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
            "device={} source={} outcome=retry_scheduled failures={} delay_ms={}",
            redacted(*address),
            ConnectionSource::AutoRetry.label(),
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
        active_card_profile, adopt_sink_wait, adoption_decision, adoption_retry_pending, audio,
        begin_device_action, card_for, clear_adoption_retry, complete_device_action_within,
        finish_device_action, parse_address, parse_daemon_request, record_adoption_sink_absent,
        record_adoption_skip_blocked, record_adoption_skip_unverified, redacted, redacted_sink,
        register_adoption_retry, running_sink_except, runtime, set_device_action_state,
        source_in_use, source_index_matching, tolerated_profile_connect, transient_connect_error,
        watch_action, Address, AdoptionDecision, AdoptionRetries, AudioAdoption, AudioRepairGuard,
        ConnectionFlightGuard, DaemonAction, DaemonCommand, DeviceActionTimeout, Duration,
        ErrorKind, Instant, ReadResult, Result, RetryState, WatchAction, ADOPTION_RETRY_WINDOW,
        ADOPT_TRIGGER_RETRY, DEVICE_ACTION_STATE, PROFILE_CONNECT_TIMEOUT,
    };
    use qol_audio::attempts::record::{Lifetime, Ownership, OwnershipState};
    use qol_audio::control::{Card, Sink, Source, SourceOutput};
    use qol_audio::devices::{Identity, State};
    use qol_runtime::protocol::DaemonRequest;
    use std::collections::{BTreeMap, HashMap};

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
            ("reclaim_device", DaemonAction::ReclaimDevice),
            ("remove_device", DaemonAction::RemoveDevice),
            ("start_search", DaemonAction::StartSearch),
            ("stop_search", DaemonAction::StopSearch),
            ("devices", DaemonAction::Devices),
            ("managed_device_options", DaemonAction::ManagedDeviceOptions),
            ("adapter_options", DaemonAction::AdapterOptions),
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
        let payload = super::adapter_status_payload(Some(&super::AdapterHealth {
            name: "hci0".into(),
            address: "AA:BB:CC:DD:EE:FF".into(),
            powered: true,
        }));
        assert_eq!(
            payload,
            serde_json::json!({
                "available": true,
                "powered": true,
            })
        );
    }

    #[test]
    fn unavailable_adapter_status_is_immediate_and_explicit() {
        assert_eq!(
            super::adapter_status_payload(None),
            serde_json::json!({
                "available": false,
                "powered": false,
            })
        );
    }

    #[test]
    fn unavailable_adapter_maps_device_actions_to_stable_failures() {
        let address = parse_address("AA:BB:CC:DD:EE:FF").unwrap();
        assert_eq!(
            super::unavailable_device_command(DaemonCommand::Connect(address)),
            Some((address, "Connect", "connect"))
        );
        assert_eq!(
            super::unavailable_device_command(DaemonCommand::SetAdapterPower(true)),
            None
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
        begin_device_action(address, "Connecting").unwrap();
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
            (ErrorKind::Failed, "br-connection-create-socket", true),
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

    fn card(index: u32, name: &str, active_profile: Option<&str>) -> Card {
        Card {
            index,
            name: name.to_owned(),
            description: name.to_owned(),
            driver: None,
            active_profile: active_profile.map(str::to_owned),
            profiles: Vec::new(),
        }
    }

    fn sink(index: u32, name: &str, state: State) -> Sink {
        Sink {
            index,
            name: name.to_owned(),
            description: name.to_owned(),
            state,
            monitor_source_index: None,
            card_index: None,
            active_port: None,
            properties: BTreeMap::new(),
            muted: false,
        }
    }

    fn source(index: u32, name: &str) -> Source {
        Source {
            index,
            name: name.to_owned(),
            description: name.to_owned(),
            state: State::Running,
            monitor_of_sink_index: None,
            card_index: None,
            active_port: None,
            properties: BTreeMap::new(),
            muted: false,
        }
    }

    fn monitor_source(index: u32, name: &str) -> Source {
        Source {
            monitor_of_sink_index: Some(324),
            ..source(index, name)
        }
    }

    fn source_output(index: u32, source_index: u32) -> SourceOutput {
        SourceOutput {
            index,
            name: format!("input stream {index}"),
            source_index,
            client_index: None,
            corked: false,
            properties: BTreeMap::new(),
        }
    }

    #[test]
    fn finds_an_exact_pipewire_bluetooth_card() {
        let cards = [
            card(43, "alsa_card.pci", None),
            card(555, "bluez_card.74_68_59_7F_5F_E9", Some("a2dp-sink")),
        ];
        assert!(card_for(&cards, "bluez_card.74_68_59_7F_5F_E9").is_some());
        assert!(card_for(&cards, "bluez_card.88_0E_85_16_CA_67").is_none());
    }

    #[test]
    fn active_card_profile_reads_the_requested_card() {
        let cards = [
            card(
                0,
                "alsa_card.pci-0000_01_00.1",
                Some("output:analog-stereo"),
            ),
            card(1, "bluez_card.74_68_59_7F_5F_E9", Some("headset-head-unit")),
            card(2, "bluez_card.11_22_33_44_55_66", None),
        ];
        assert_eq!(
            active_card_profile(&cards, "bluez_card.74_68_59_7F_5F_E9").as_deref(),
            Some("headset-head-unit")
        );
        assert_eq!(
            active_card_profile(&cards, "alsa_card.pci-0000_01_00.1").as_deref(),
            Some("output:analog-stereo")
        );
        assert_eq!(
            active_card_profile(&cards, "bluez_card.11_22_33_44_55_66"),
            None
        );
        assert_eq!(
            active_card_profile(&cards, "bluez_card.99_99_99_99_99_99"),
            None
        );
    }

    #[test]
    fn microphone_source_lookup_matches_only_the_requested_device() {
        let sources = [
            source(160, "bluez_input.74_68_59_7F_5F_E9.0"),
            monitor_source(161, "bluez_output.74_68_59_7F_5F_E9.1.monitor"),
            source(162, "bluez_input.88_0E_85_16_CA_67.0"),
        ];
        assert_eq!(
            source_index_matching(&sources, "bluez_input.74_68_59_7F_5F_E9.0"),
            Some(160)
        );
        assert_eq!(
            source_index_matching(&sources, "bluez_input.11_22_33_44_55_66.0"),
            None
        );
        assert_eq!(source_index_matching(&sources, "alsa_input.monitor"), None);
    }

    #[test]
    fn microphone_is_in_use_only_when_a_stream_targets_its_source() {
        let outputs = [source_output(7, 160), source_output(8, 201)];
        assert!(source_in_use(&outputs, 160));
        assert!(source_in_use(&outputs, 201));
        assert!(!source_in_use(&outputs, 205));
        assert!(!source_in_use(&[], 160));
    }

    #[test]
    fn running_sink_except_ignores_own_prefix_and_non_running_rows() {
        const OWN: &str = "bluez_output.74_68_59_7F_5F_E9";
        let cases: [(Vec<Sink>, Option<&str>); 6] = [
            (Vec::new(), None),
            (
                vec![
                    sink(53, "alsa_output.pci.analog-stereo", State::Suspended),
                    sink(72, "alsa_output.pci.hdmi-stereo", State::Idle),
                ],
                None,
            ),
            (
                vec![sink(75, "bluez_output.74_68_59_7F_5F_E9.1", State::Running)],
                None,
            ),
            (
                vec![
                    sink(75, "bluez_output.74_68_59_7F_5F_E9.1", State::Running),
                    sink(8842, "bluez_output.74_68_59_7F_5F_E0.1", State::Running),
                ],
                Some("bluez_output.74_68_59_7F_5F_E0.1"),
            ),
            (
                vec![
                    sink(58, "alsa_output.usb-hyperx.analog-stereo", State::Running),
                    sink(75, "bluez_output.74_68_59_7F_5F_E9.1", State::Suspended),
                ],
                Some("alsa_output.usb-hyperx.analog-stereo"),
            ),
            (
                vec![sink(75, "bluez_output.74_68_59_7F_5F_E9.1", State::Unknown)],
                None,
            ),
        ];
        for (sinks, expected) in cases {
            assert_eq!(running_sink_except(&sinks, OWN).as_deref(), expected);
        }
    }

    #[test]
    fn audio_repairs_run_one_at_a_time_per_device() {
        let first = parse_address("AA:BB:CC:DD:EE:FF").unwrap();
        let second = parse_address("11:22:33:44:55:66").unwrap();
        let held = AudioRepairGuard::acquire(first).expect("first repair should acquire");
        assert!(
            AudioRepairGuard::acquire(first).is_none(),
            "a second repair for the same device must be refused"
        );
        assert!(
            AudioRepairGuard::acquire(second).is_some(),
            "a repair for another device must proceed"
        );
        std::mem::drop(held);
        assert!(
            AudioRepairGuard::acquire(first).is_some(),
            "the device must be repairable again once the guard drops"
        );
    }

    #[test]
    fn connection_flights_cover_reconnect_and_profile_repair_for_one_device() {
        let address = parse_address("AA:BB:CC:DD:EE:FF").unwrap();
        let held = ConnectionFlightGuard::acquire(address).expect("flight should acquire");
        assert!(
            ConnectionFlightGuard::acquire(address).is_none(),
            "a profile repair must not race an explicit or automatic reconnect"
        );
        assert!(ConnectionFlightGuard::active(address));
        drop(held);
        assert!(!ConnectionFlightGuard::active(address));
    }

    #[test]
    fn default_output_lookup_resolves_only_the_requested_bluetooth_sink() {
        let sinks = [
            sink(
                50,
                "alsa_output.pci-0000_01_00.1.hdmi-stereo",
                State::Suspended,
            ),
            sink(324, "bluez_output.AA_BB_CC_DD_EE_FF.1", State::Suspended),
        ];
        let cases = [
            (
                "bluez_output.AA_BB_CC_DD_EE_FF",
                Some("bluez_output.AA_BB_CC_DD_EE_FF.1"),
            ),
            ("bluez_output.11_22_33_44_55_66", None),
        ];
        for (prefix, expected) in cases {
            assert_eq!(
                audio::sink_matching(&sinks, prefix).map(|sink| sink.name.as_str()),
                expected,
                "prefix: {prefix}"
            );
        }
    }

    #[test]
    fn adoption_decision_covers_each_gate() {
        let keep = adoption_decision(
            AudioAdoption::Keep,
            true,
            Ok(OwnershipState::None),
            Ok(None),
        );
        assert_eq!(keep, AdoptionDecision::SkipKeep);
        let disabled = adoption_decision(
            AudioAdoption::Adopt,
            false,
            Ok(OwnershipState::None),
            Ok(None),
        );
        assert_eq!(disabled, AdoptionDecision::SkipDisabled);
        let unverified = adoption_decision(
            AudioAdoption::Adopt,
            true,
            Ok(OwnershipState::None),
            Err(anyhow::anyhow!("the PipeWire sink listing is unavailable")),
        );
        assert_eq!(unverified, AdoptionDecision::SkipUnverified);
        let blocked = adoption_decision(
            AudioAdoption::Adopt,
            true,
            Ok(OwnershipState::None),
            Ok(Some("alsa_output.hdmi-stereo".into())),
        );
        assert_eq!(
            blocked,
            AdoptionDecision::SkipBlocked("alsa_output.hdmi-stereo".into())
        );
        let adopt = adoption_decision(
            AudioAdoption::Adopt,
            true,
            Ok(OwnershipState::None),
            Ok(None),
        );
        assert_eq!(adopt, AdoptionDecision::Adopt);
        let sound_owns_the_default = adoption_decision(
            AudioAdoption::Adopt,
            true,
            Ok(OwnershipState::Held(Ownership {
                owner: "plugin-sound".into(),
                epoch: 1,
                lifetime: Lifetime::ResidentPolicy,
                output: Identity::from_raw("bluez_output.AA_BB_CC_DD_EE_FF.1"),
            })),
            Ok(None),
        );
        assert_eq!(
            sound_owns_the_default,
            AdoptionDecision::SkipOwned("plugin-sound".into())
        );
        let unreadable_ownership_fails_closed = adoption_decision(
            AudioAdoption::Adopt,
            true,
            Ok(OwnershipState::Unreadable(
                "the ownership record is corrupt".into(),
            )),
            Ok(None),
        );
        assert_eq!(
            unreadable_ownership_fails_closed,
            AdoptionDecision::SkipUnverified
        );
    }

    #[test]
    fn the_watch_tick_retries_a_pending_adoption_and_repairs_before_it() {
        let cases = [
            (true, true, WatchAction::Repair),
            (true, false, WatchAction::Repair),
            (false, true, WatchAction::Retry),
            (false, false, WatchAction::Idle),
        ];
        for (degraded, pending, expected) in cases {
            assert_eq!(
                watch_action(degraded, pending),
                expected,
                "degraded={degraded} pending={pending}"
            );
        }
    }

    #[test]
    fn a_retry_never_waits_for_a_sink_the_next_tick_can_find() {
        assert_eq!(adopt_sink_wait(ADOPT_TRIGGER_RETRY), Duration::ZERO);
        assert_eq!(adopt_sink_wait("connect"), PROFILE_CONNECT_TIMEOUT);
        assert_eq!(adopt_sink_wait("repair"), PROFILE_CONNECT_TIMEOUT);
    }

    #[test]
    fn a_pending_adoption_expires_after_the_retry_window() {
        let address = parse_address("AA:BB:CC:DD:EE:FF").unwrap();
        let now = Instant::now();
        let window = ADOPTION_RETRY_WINDOW;
        let mut retries = AdoptionRetries::default();
        assert!(retries.register(address, now));
        assert!(!retries.register(address, now + Duration::from_secs(120)));
        assert!(retries
            .expire(now + window - Duration::from_secs(1))
            .is_empty());
        assert_eq!(retries.expire(now + window), vec![address]);
        assert!(!retries.is_pending(address));
        assert!(retries.expire(now + window).is_empty());
    }

    #[test]
    fn a_device_disconnect_drops_only_its_pending_adoption() {
        let first = parse_address("AA:BB:CC:DD:EE:FF").unwrap();
        let second = parse_address("11:22:33:44:55:66").unwrap();
        let now = Instant::now();
        let mut retries = AdoptionRetries::default();
        retries.register(first, now);
        retries.register(second, now);
        retries.clear(first);
        assert!(!retries.is_pending(first));
        assert!(retries.is_pending(second));
    }

    #[test]
    fn a_recorded_skip_leaves_the_device_pending_until_it_is_cleared() {
        let address = parse_address("0A:0B:0C:0D:0E:01").unwrap();
        clear_adoption_retry(address);
        record_adoption_skip_blocked(address, "alsa_output.pci.hdmi-stereo");
        assert!(adoption_retry_pending(address));
        record_adoption_skip_unverified(address);
        record_adoption_sink_absent(address);
        assert!(adoption_retry_pending(address));
        clear_adoption_retry(address);
        assert!(!adoption_retry_pending(address));
    }

    #[test]
    fn only_the_first_skip_of_a_pending_device_is_recorded() {
        let address = parse_address("0A:0B:0C:0D:0E:02").unwrap();
        clear_adoption_retry(address);
        assert!(register_adoption_retry(address));
        assert!(!register_adoption_retry(address));
        clear_adoption_retry(address);
        assert!(register_adoption_retry(address));
        clear_adoption_retry(address);
    }

    #[test]
    fn redacted_sink_masks_all_but_the_last_two_octets() {
        assert_eq!(
            redacted_sink("bluez_output.74_68_59_7F_5F_E9.1"),
            "bluez_output.**:**:**:**:5F:E9.1"
        );
        assert_eq!(
            redacted_sink("bluez_output.88_0E_85_16_CA_67.2"),
            "bluez_output.**:**:**:**:CA:67.2"
        );
        assert_eq!(
            redacted_sink("alsa_output.usb-hyperx.analog-stereo"),
            "alsa_output.usb-hyperx.analog-stereo"
        );
        assert_eq!(redacted_sink("null"), "null");
        assert_eq!(
            redacted_sink("bluez_output.not_a_mac.1"),
            "bluez_output.not_a_mac.1"
        );
        assert_eq!(
            redacted_sink("bluez_output.AA_BB_CC_DD_EE"),
            "bluez_output.AA_BB_CC_DD_EE"
        );
    }

    #[test]
    fn redacted_addresses_never_carry_the_vendor_prefix() {
        let cases = [
            (
                Address([0x74, 0x68, 0x59, 0x7F, 0x5F, 0xE9]),
                "**:**:**:**:5F:E9",
                "74:68:59",
            ),
            (
                Address([0x88, 0x0E, 0x85, 0x16, 0xCA, 0x67]),
                "**:**:**:**:CA:67",
                "88:0E:85",
            ),
            (Address([0; 6]), "**:**:**:**:00:00", "00:00:00"),
            (Address([0xFF; 6]), "**:**:**:**:FF:FF", "FF:FF:FF"),
        ];
        for (address, expected, vendor) in cases {
            let masked = redacted(address);
            assert_eq!(masked, expected, "case: {expected}");
            assert!(!masked.contains(vendor), "case: {expected}");
        }
    }
}
