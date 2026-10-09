use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use qol_audio::bluetooth::BluetoothEndpoint;
use qol_headless::DoctorCheckResult;
use windows::core::{Owned, GUID, HRESULT};
use windows::Devices::Radios::{Radio, RadioAccessStatus, RadioKind, RadioState};
use windows::Win32::Devices::Bluetooth::{
    BluetoothAuthenticateDeviceEx, BluetoothEnumerateInstalledServices, BluetoothFindFirstDevice,
    BluetoothFindFirstRadio, BluetoothFindNextDevice, BluetoothGetDeviceInfo,
    BluetoothGetRadioInfo, BluetoothRemoveDevice, BluetoothSetServiceState,
    MITMProtectionNotDefined, BLUETOOTH_ADDRESS, BLUETOOTH_ADDRESS_0, BLUETOOTH_DEVICE_INFO,
    BLUETOOTH_DEVICE_SEARCH_PARAMS, BLUETOOTH_FIND_RADIO_PARAMS, BLUETOOTH_RADIO_INFO,
    BLUETOOTH_SERVICE_DISABLE, BLUETOOTH_SERVICE_ENABLE,
};
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, HANDLE};

use crate::bluetooth::{
    connection_ready, normalize_address, AdapterHealth, BackendCapabilities, DeviceInfo,
    ReconnectReport, ReconnectSelection,
};
use crate::config::ReconnectConfig;

use super::paired::{self, PairedStack};
pub use super::paired::{search_status_snapshot, settings_action, settings_query, stop_search};

pub const CAPABILITIES: BackendCapabilities = BackendCapabilities {
    separate_trust_flag: false,
    audio_reclaim: crate::audio_claim::platform::RECLAIM_SUPPORTED,
};

const TRUST_UNSUPPORTED: &str =
    "Windows pairs and trusts in one step, so there is no separate trust to change";
const SEARCH_TIMEOUT_MULTIPLIER: u8 = 8;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const POWER_TIMEOUT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(250);
const RETRY_SPACING: Duration = Duration::from_secs(2);
const HID_SERVICE: GUID = GUID::from_u128(0x00001124_0000_1000_8000_00805f9b34fb);

struct WindowsStack;

impl PairedStack for WindowsStack {
    const TRUST_REFUSAL: &'static str = TRUST_UNSUPPORTED;
    const RETRY_SPACING: Duration = RETRY_SPACING;

    fn paired_devices() -> Vec<DeviceInfo> {
        list_devices().unwrap_or_else(|error| {
            log::warn!("Bluetooth devices could not be listed: {error:#}");
            Vec::new()
        })
    }

    fn adapter_health() -> Result<AdapterHealth> {
        adapter_health()
    }

    fn set_adapter_powered(powered: bool) -> Result<AdapterHealth> {
        set_adapter_powered(powered)
    }

    fn connect_device(address: &str, power_on_adapter: bool) -> Result<DeviceInfo> {
        connect_device(address, power_on_adapter)
    }

    fn disconnect_device(address: &str) -> Result<DeviceInfo> {
        disconnect_device(address)
    }

    fn pair_device(address: &str, power_on_adapter: bool) -> Result<DeviceInfo> {
        pair_device(address, power_on_adapter)
    }

    fn remove_device(address: &str) -> Result<()> {
        remove_device(address)
    }

    fn search_devices(config: &ReconnectConfig) -> Result<Vec<DeviceInfo>> {
        search_devices(config)
    }

    fn pause(slice: Duration) {
        std::thread::sleep(slice);
    }
}

fn wait_until(timeout: Duration, mut satisfied: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if satisfied() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

fn win32_reason(code: u32) -> String {
    if code == ERROR_ACCESS_DENIED.0 {
        return format!("error {code}: Windows lets only an administrator change this");
    }
    format!("error {code}")
}

fn address_text(raw: u64) -> String {
    raw.to_be_bytes()[2..]
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn address_value(address: &str) -> Result<u64> {
    let normalized = normalize_address(address)?;
    u64::from_str_radix(&normalized.replace(':', ""), 16)
        .with_context(|| format!("{normalized} is not a Bluetooth address"))
}

fn name_text(units: &[u16]) -> String {
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end]).trim().to_owned()
}

fn uuid_text(value: u128) -> String {
    let hex = format!("{value:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

fn audio_state(address: &str, endpoints: &[BluetoothEndpoint]) -> Option<bool> {
    let mut matching = endpoints
        .iter()
        .filter(|endpoint| endpoint.address == address)
        .peekable();
    matching.peek()?;
    Some(matching.any(|endpoint| endpoint.active))
}

fn audio_endpoints() -> Vec<BluetoothEndpoint> {
    qol_audio::bluetooth::endpoints().unwrap_or_else(|error| {
        log::debug!("Bluetooth audio endpoints could not be read: {error}");
        Vec::new()
    })
}

fn blank_record() -> BLUETOOTH_DEVICE_INFO {
    BLUETOOTH_DEVICE_INFO {
        dwSize: size_of::<BLUETOOTH_DEVICE_INFO>() as u32,
        ..Default::default()
    }
}

fn search_params(inquiry: bool) -> BLUETOOTH_DEVICE_SEARCH_PARAMS {
    BLUETOOTH_DEVICE_SEARCH_PARAMS {
        dwSize: size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32,
        fReturnAuthenticated: true.into(),
        fReturnRemembered: true.into(),
        fReturnUnknown: inquiry.into(),
        fReturnConnected: true.into(),
        fIssueInquiry: inquiry.into(),
        cTimeoutMultiplier: if inquiry {
            SEARCH_TIMEOUT_MULTIPLIER
        } else {
            0
        },
        hRadio: HANDLE::default(),
    }
}

fn find_records(inquiry: bool) -> Result<Vec<BLUETOOTH_DEVICE_INFO>> {
    let params = search_params(inquiry);
    let mut first = blank_record();
    let find = match unsafe { BluetoothFindFirstDevice(&params, &mut first) } {
        Ok(find) => unsafe { Owned::new(find) },
        Err(error) if error.code() == HRESULT::from_win32(ERROR_NO_MORE_ITEMS.0) => {
            return Ok(Vec::new())
        }
        Err(error) => bail!("Windows could not list Bluetooth devices: {error}"),
    };
    let mut records = vec![first];
    loop {
        let mut next = blank_record();
        if unsafe { BluetoothFindNextDevice(*find, &mut next) }.is_err() {
            return Ok(records);
        }
        records.push(next);
    }
}

fn record(address: &str) -> Result<BLUETOOTH_DEVICE_INFO> {
    let mut record = blank_record();
    record.Address = BLUETOOTH_ADDRESS {
        Anonymous: BLUETOOTH_ADDRESS_0 {
            ullLong: address_value(address)?,
        },
    };
    let status = unsafe { BluetoothGetDeviceInfo(None, &mut record) };
    if status != ERROR_SUCCESS.0 {
        bail!(
            "Windows knows no Bluetooth device at {address} ({})",
            win32_reason(status)
        );
    }
    Ok(record)
}

fn installed_services(record: &BLUETOOTH_DEVICE_INFO) -> Vec<GUID> {
    let mut count = 0u32;
    unsafe { BluetoothEnumerateInstalledServices(None, record, &mut count, None) };
    if count == 0 {
        return Vec::new();
    }
    let mut services = vec![GUID::zeroed(); count as usize];
    let status = unsafe {
        BluetoothEnumerateInstalledServices(None, record, &mut count, Some(services.as_mut_ptr()))
    };
    if status != ERROR_SUCCESS.0 {
        return Vec::new();
    }
    services.truncate(count as usize);
    services
}

fn device_info(record: &BLUETOOTH_DEVICE_INFO, endpoints: &[BluetoothEndpoint]) -> DeviceInfo {
    let address = address_text(unsafe { record.Address.Anonymous.ullLong });
    let name = name_text(&record.szName);
    let uuids = installed_services(record)
        .iter()
        .map(|service| uuid_text(service.to_u128()))
        .collect::<Vec<_>>();
    let paired = record.fAuthenticated.as_bool() || record.fRemembered.as_bool();
    DeviceInfo {
        alias: if name.is_empty() {
            address.clone()
        } else {
            name
        },
        paired,
        trusted: paired,
        connected: record.fConnected.as_bool(),
        audio_connected: audio_state(&address, endpoints),
        services_resolved: !uuids.is_empty(),
        icon: None,
        class: Some(record.ulClassofDevice),
        uuids,
        rssi: None,
        address,
    }
}

fn device(address: &str) -> Result<DeviceInfo> {
    Ok(device_info(&record(address)?, &audio_endpoints()))
}

fn bluetooth_radio() -> Result<Option<Radio>> {
    for radio in Radio::GetRadiosAsync()?.get()? {
        if radio.Kind()? == RadioKind::Bluetooth {
            return Ok(Some(radio));
        }
    }
    Ok(None)
}

fn radio_info() -> Option<BLUETOOTH_RADIO_INFO> {
    let params = BLUETOOTH_FIND_RADIO_PARAMS {
        dwSize: size_of::<BLUETOOTH_FIND_RADIO_PARAMS>() as u32,
    };
    let mut handle = HANDLE::default();
    let find = unsafe { BluetoothFindFirstRadio(&params, &mut handle) }.ok()?;
    let _find = unsafe { Owned::new(find) };
    let handle = unsafe { Owned::new(handle) };
    let mut info = BLUETOOTH_RADIO_INFO {
        dwSize: size_of::<BLUETOOTH_RADIO_INFO>() as u32,
        ..Default::default()
    };
    (unsafe { BluetoothGetRadioInfo(*handle, &mut info) } == ERROR_SUCCESS.0).then_some(info)
}

fn restart_input_service(record: &BLUETOOTH_DEVICE_INFO, alias: &str) -> Result<()> {
    if !installed_services(record).contains(&HID_SERVICE) {
        bail!(
            "Windows has no call that connects {alias}: it has no Bluetooth audio endpoint and no input service, so it has to reconnect itself"
        );
    }
    let disabled =
        unsafe { BluetoothSetServiceState(None, record, &HID_SERVICE, BLUETOOTH_SERVICE_DISABLE) };
    let enabled =
        unsafe { BluetoothSetServiceState(None, record, &HID_SERVICE, BLUETOOTH_SERVICE_ENABLE) };
    service_restart(alias, disabled, enabled)
}

fn service_restart(alias: &str, disabled: u32, enabled: u32) -> Result<()> {
    if enabled != ERROR_SUCCESS.0 {
        bail!(
            "Windows did not turn the input service of {alias} back on ({}); run connect again to retry",
            win32_reason(enabled)
        );
    }
    if disabled != ERROR_SUCCESS.0 {
        bail!(
            "Windows refused to restart the input service of {alias} ({})",
            win32_reason(disabled)
        );
    }
    Ok(())
}

pub fn required_binaries_check() -> DoctorCheckResult {
    match adapter_health() {
        Ok(health) => DoctorCheckResult::ok(
            "required_binaries",
            "The Windows Bluetooth stack reports a radio",
        )
        .with_details(serde_json::json!({
            "platform": "windows",
            "radio": true,
            "powered": health.powered,
            "executed": false,
        })),
        Err(error) => DoctorCheckResult::fail(
            "required_binaries",
            format!("Windows reported no Bluetooth radio: {error:#}"),
        )
        .with_fix("Check that this PC has a Bluetooth adapter and that its driver is installed and enabled in Device Manager")
        .with_details(serde_json::json!({
            "platform": "windows",
            "radio": false,
            "executed": false,
        })),
    }
}

pub fn list_devices() -> Result<Vec<DeviceInfo>> {
    let endpoints = audio_endpoints();
    Ok(find_records(false)?
        .iter()
        .map(|record| device_info(record, &endpoints))
        .collect())
}

pub fn adapter_health() -> Result<AdapterHealth> {
    let radio = bluetooth_radio().unwrap_or_else(|error| {
        log::debug!("Windows radio state could not be read: {error}");
        None
    });
    let info = radio_info();
    if radio.is_none() && info.is_none() {
        bail!("this PC has no Bluetooth adapter");
    }
    let powered = match &radio {
        Some(radio) => radio.State()? == RadioState::On,
        None => true,
    };
    let name = info
        .as_ref()
        .map(|info| name_text(&info.szName))
        .filter(|name| !name.is_empty())
        .or_else(|| {
            radio
                .as_ref()
                .and_then(|radio| radio.Name().ok())
                .map(|name| name.to_string())
        })
        .unwrap_or_else(|| "Bluetooth".to_string());
    let address = info
        .map(|info| address_text(unsafe { info.address.Anonymous.ullLong }))
        .unwrap_or_default();
    Ok(AdapterHealth {
        name,
        address,
        powered,
    })
}

fn change_radio_state(powered: bool) -> Result<()> {
    let access = Radio::RequestAccessAsync()?.get()?;
    if access != RadioAccessStatus::Allowed {
        bail!(
            "Windows does not let this app change the Bluetooth radio (access status {})",
            access.0
        );
    }
    let radio = bluetooth_radio()?.ok_or_else(|| anyhow!("Windows reports no Bluetooth radio"))?;
    let target = if powered {
        RadioState::On
    } else {
        RadioState::Off
    };
    let status = radio.SetStateAsync(target)?.get()?;
    if status != RadioAccessStatus::Allowed {
        bail!(
            "Windows refused to change the Bluetooth radio (access status {})",
            status.0
        );
    }
    Ok(())
}

pub fn set_adapter_powered(powered: bool) -> Result<AdapterHealth> {
    let result = change_radio_state(powered);
    let outcome = if result.is_ok() { "ok" } else { "failed" };
    qol_runtime::probe!(
        "BLUETOOTH_ADAPTER_POWER",
        "source=cli powered={powered} outcome={outcome}"
    );
    result?;
    wait_until(POWER_TIMEOUT, || {
        adapter_health().is_ok_and(|health| health.powered == powered)
    });
    adapter_health()
}

pub fn connect_device(address: &str, power_on_adapter: bool) -> Result<DeviceInfo> {
    paired::ensure_powered::<WindowsStack>(power_on_adapter)?;
    let record = record(address)?;
    let current = device_info(&record, &audio_endpoints());
    if connection_ready(&current) {
        return Ok(current);
    }
    let asked = qol_audio::bluetooth::reconnect(&current.address)
        .with_context(|| format!("Windows could not ask {} to reconnect", current.alias))?;
    if asked == 0 {
        restart_input_service(&record, &current.alias)?;
    }
    if !wait_until(CONNECT_TIMEOUT, || {
        device(address).is_ok_and(|device| connection_ready(&device))
    }) {
        bail!("{} did not finish connecting", current.alias);
    }
    device(address)
}

pub fn disconnect_device(address: &str) -> Result<DeviceInfo> {
    let current = device(address)?;
    if !current.connected {
        return Ok(current);
    }
    let asked = qol_audio::bluetooth::disconnect(&current.address)
        .with_context(|| format!("Windows could not ask {} to disconnect", current.alias))?;
    if asked == 0 {
        bail!(
            "Windows has no call that disconnects {} without removing the pairing; it disconnects only Bluetooth audio devices",
            current.alias
        );
    }
    device(address)
}

pub fn pair_device(address: &str, power_on_adapter: bool) -> Result<DeviceInfo> {
    paired::ensure_powered::<WindowsStack>(power_on_adapter)?;
    let mut record = record(address)?;
    if record.fAuthenticated.as_bool() {
        return Ok(device_info(&record, &audio_endpoints()));
    }
    let status = unsafe {
        BluetoothAuthenticateDeviceEx(None, None, &mut record, None, MITMProtectionNotDefined)
    };
    if status != ERROR_SUCCESS.0 {
        bail!(
            "Windows could not pair with {address} ({})",
            win32_reason(status)
        );
    }
    device(address)
}

pub fn set_device_trusted(_address: &str, _trusted: bool) -> Result<DeviceInfo> {
    bail!(TRUST_UNSUPPORTED)
}

pub fn remove_device(address: &str) -> Result<()> {
    let target = BLUETOOTH_ADDRESS {
        Anonymous: BLUETOOTH_ADDRESS_0 {
            ullLong: address_value(address)?,
        },
    };
    let status = unsafe { BluetoothRemoveDevice(&target) };
    if status != ERROR_SUCCESS.0 {
        bail!(
            "Windows refused to remove {address} ({})",
            win32_reason(status)
        );
    }
    Ok(())
}

pub fn search_devices(config: &ReconnectConfig) -> Result<Vec<DeviceInfo>> {
    paired::ensure_powered::<WindowsStack>(config.power_on_adapter)?;
    paired::mark_search_starting()?;
    let records = match find_records(true) {
        Ok(records) => records,
        Err(error) => {
            paired::reset_discovery_state()?;
            return Err(error);
        }
    };
    let endpoints = audio_endpoints();
    let found = records
        .iter()
        .map(|record| device_info(record, &endpoints))
        .collect::<Vec<_>>();
    if paired::searching()? {
        for device in &found {
            paired::record_discovered_device(device.clone())?;
        }
    }
    paired::mark_search_stopped()?;
    qol_runtime::probe!("BLUETOOTH_SEARCH", "stage=stop found={}", found.len());
    Ok(found)
}

pub fn reconnect_devices(
    config: &ReconnectConfig,
    selection: ReconnectSelection,
) -> Result<ReconnectReport> {
    paired::reconnect_devices::<WindowsStack>(config, selection)
}

pub fn devices_snapshot() -> Result<serde_json::Value> {
    paired::devices_snapshot::<WindowsStack>()
}

pub fn run_daemon(config: ReconnectConfig) -> Result<()> {
    paired::run_daemon::<WindowsStack>(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_audio::devices::Direction;

    fn endpoint(address: &str, active: bool) -> BluetoothEndpoint {
        BluetoothEndpoint {
            address: address.to_string(),
            direction: Direction::Output,
            active,
        }
    }

    #[test]
    fn addresses_round_trip_between_windows_and_the_domain() {
        let cases = [
            (0x7468_597f_5fe9_u64, "74:68:59:7F:5F:E9"),
            (0x0000_0000_0001_u64, "00:00:00:00:00:01"),
            (0xffff_ffff_ffff_u64, "FF:FF:FF:FF:FF:FF"),
        ];
        for (raw, text) in cases {
            assert_eq!(address_text(raw), text, "{raw:x}");
            assert_eq!(address_value(text).unwrap(), raw, "{text}");
            assert_eq!(address_value(&text.to_lowercase()).unwrap(), raw, "{text}");
        }
    }

    #[test]
    fn a_device_name_stops_at_the_first_nul() {
        let cases = [
            ("Luna 2\0garbage", "Luna 2"),
            ("  WH-1000XM4 ", "WH-1000XM4"),
            ("", ""),
            ("no terminator", "no terminator"),
        ];
        for (raw, expected) in cases {
            let units = raw.encode_utf16().collect::<Vec<_>>();
            assert_eq!(name_text(&units), expected, "{raw:?}");
        }
    }

    #[test]
    fn service_uuids_read_like_the_other_backends() {
        let cases = [
            (
                0x0000110b_0000_1000_8000_00805f9b34fb_u128,
                "0000110b-0000-1000-8000-00805f9b34fb",
            ),
            (
                HID_SERVICE.to_u128(),
                "00001124-0000-1000-8000-00805f9b34fb",
            ),
        ];
        for (value, text) in cases {
            assert_eq!(uuid_text(value), text);
        }
    }

    #[test]
    fn audio_is_known_only_for_devices_with_an_audio_endpoint() {
        let endpoints = [
            endpoint("74:68:59:7F:5F:E9", false),
            endpoint("74:68:59:7F:5F:E9", true),
            endpoint("04:FF:AA:BB:CC:DD", false),
        ];
        let cases = [
            ("74:68:59:7F:5F:E9", Some(true)),
            ("04:FF:AA:BB:CC:DD", Some(false)),
            ("00:11:22:33:44:55", None),
        ];
        for (address, expected) in cases {
            assert_eq!(audio_state(address, &endpoints), expected, "{address}");
        }
    }

    #[test]
    fn a_service_restart_reports_the_step_that_failed() {
        let denied = ERROR_ACCESS_DENIED.0;
        let cases = [
            (ERROR_SUCCESS.0, ERROR_SUCCESS.0, None),
            (denied, ERROR_SUCCESS.0, Some("refused to restart")),
            (ERROR_SUCCESS.0, 87, Some("back on")),
            (denied, denied, Some("administrator")),
        ];
        for (disabled, enabled, expected) in cases {
            let outcome = service_restart("Pad", disabled, enabled);
            match expected {
                None => assert!(outcome.is_ok(), "{disabled} {enabled}"),
                Some(fragment) => {
                    let message = format!("{:#}", outcome.unwrap_err());
                    assert!(message.contains(fragment), "{message}");
                }
            }
        }
    }
}
