use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};

use qol_windowing::display::{
    validate_layout, DisplayEnumerator, DisplayError, DisplayHandle, DisplayMode, DisplayOps,
    DisplayPlacement, DisplaySnapshot,
};
use qol_windowing::MonitorBounds;
use windows_sys::Win32::Foundation::{ERROR_SUCCESS, POINTL};
use windows_sys::Win32::Graphics::Gdi::{
    ChangeDisplaySettingsExW, EnumDisplayDevicesW, EnumDisplaySettingsExW, CDS_NORESET,
    CDS_SET_PRIMARY, CDS_TEST, CDS_TYPE, CDS_UPDATEREGISTRY, DEVMODEW, DISPLAY_DEVICEW,
    DISPLAY_DEVICE_ATTACHED_TO_DESKTOP, DISPLAY_DEVICE_MIRRORING_DRIVER,
    DISPLAY_DEVICE_PRIMARY_DEVICE, DISP_CHANGE_SUCCESSFUL, DM_BITSPERPEL, DM_DISPLAYFREQUENCY,
    DM_PELSHEIGHT, DM_PELSWIDTH, DM_POSITION, ENUM_CURRENT_SETTINGS,
};
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_BINARY};
use windows_sys::Win32::UI::WindowsAndMessaging::EDD_GET_DEVICE_INTERFACE_NAME;

use crate::monitor::backends::gdi_display::{
    connector_from_device, device_instance_from_interface, display_change_reason, distinct_modes,
    edid_registry_key, identity_from, mode_from_setting, pick_setting, primary_anchored,
    DisplaySetting,
};

pub(super) struct WindowsDisplay;

pub(super) struct Adapter {
    pub(super) device: String,
    primary: bool,
    interface: Option<String>,
}

impl Adapter {
    pub(super) fn connector(&self) -> &str {
        connector_from_device(&self.device)
    }

    fn handle(&self) -> DisplayHandle {
        let instance = self
            .interface
            .as_deref()
            .and_then(device_instance_from_interface);
        let edid = instance.as_deref().and_then(read_edid);
        let (id, edid_sha256, identity_unstable) =
            identity_from(instance.as_deref(), self.connector(), edid.as_deref());
        DisplayHandle::new(
            id,
            self.connector().to_string(),
            edid_sha256,
            identity_unstable,
        )
    }
}

impl DisplayEnumerator for WindowsDisplay {
    fn enumerate(&self) -> Result<Vec<DisplayHandle>, DisplayError> {
        Ok(attached_adapters().iter().map(Adapter::handle).collect())
    }

    fn snapshot(&self) -> Result<Vec<DisplaySnapshot>, DisplayError> {
        attached_adapters()
            .iter()
            .map(|adapter| {
                let current = current_settings(&adapter.device)?;
                let setting = setting_of(&current);
                let position = position_of(&current);
                Ok(DisplaySnapshot {
                    handle: adapter.handle(),
                    bounds: MonitorBounds {
                        x: position.x as f32,
                        y: position.y as f32,
                        width: setting.width as f32,
                        height: setting.height as f32,
                    },
                    primary: adapter.primary,
                    mode: Some(mode_from_setting(setting)),
                })
            })
            .collect()
    }
}

impl DisplayOps for WindowsDisplay {
    fn modes(&self, handle: &DisplayHandle) -> Result<Vec<DisplayMode>, DisplayError> {
        let adapter = adapter_for(handle, "modes")?;
        Ok(distinct_modes(&all_settings(&adapter.device)))
    }

    fn set_mode(&self, handle: &DisplayHandle, mode: &DisplayMode) -> Result<(), DisplayError> {
        let adapter = adapter_for(handle, "modes")?;
        let mut devmode = current_settings(&adapter.device)?;
        let setting = pick_setting(&all_settings(&adapter.device), mode, devmode.dmBitsPerPel)
            .ok_or_else(|| DisplayError::LayoutInvalid {
                reason: format!(
                    "mode {}x{}@{} is not offered by {}",
                    mode.width,
                    mode.height,
                    mode.refresh_hz,
                    handle.connector()
                ),
            })?;
        devmode.dmPelsWidth = setting.width;
        devmode.dmPelsHeight = setting.height;
        devmode.dmDisplayFrequency = setting.refresh_hz;
        devmode.dmBitsPerPel = setting.bits_per_pixel;
        devmode.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY | DM_BITSPERPEL;
        change(&adapter.device, &devmode, CDS_TEST)?;
        change(&adapter.device, &devmode, CDS_UPDATEREGISTRY)?;
        let after = setting_of(&current_settings(&adapter.device)?);
        if (after.width, after.height, after.refresh_hz)
            != (mode.width, mode.height, mode.refresh_hz)
        {
            return Err(DisplayError::LayoutInvalid {
                reason: format!(
                    "the mode write on {} did not verify: Windows reports {}x{}@{}, requested {}x{}@{}",
                    handle.connector(),
                    after.width,
                    after.height,
                    after.refresh_hz,
                    mode.width,
                    mode.height,
                    mode.refresh_hz
                ),
            });
        }
        Ok(())
    }

    fn set_layout(&self, placements: &[DisplayPlacement]) -> Result<(), DisplayError> {
        validate_layout(placements)?;
        let adapters = attached_adapters();
        let mut staged = Vec::new();
        for placed in primary_anchored(placements) {
            let adapter = adapters
                .iter()
                .find(|adapter| adapter.connector() == placed.connector)
                .ok_or_else(|| DisplayError::NotFound {
                    capability: "layout",
                    selector: placed.connector.clone(),
                })?;
            let original = current_settings(&adapter.device)?;
            let mut target = original;
            target.dmFields = DM_POSITION;
            let mut position = unsafe { target.Anonymous1.Anonymous2 };
            position.dmPosition = POINTL {
                x: placed.x,
                y: placed.y,
            };
            target.Anonymous1.Anonymous2 = position;
            staged.push((adapter.device.as_str(), original, target, placed.primary));
        }
        for (index, (device, _, target, primary)) in staged.iter().enumerate() {
            let flags =
                CDS_UPDATEREGISTRY | CDS_NORESET | if *primary { CDS_SET_PRIMARY } else { 0 };
            if let Err(error) = change(device, target, flags) {
                for (device, original, _, _) in &staged[..index] {
                    let mut original = *original;
                    original.dmFields = DM_POSITION;
                    let _ = change(device, &original, CDS_UPDATEREGISTRY | CDS_NORESET);
                }
                return Err(error);
            }
        }
        let code = unsafe { ChangeDisplaySettingsExW(null(), null(), null_mut(), 0, null()) };
        if code != DISP_CHANGE_SUCCESSFUL {
            return Err(DisplayError::LayoutInvalid {
                reason: format!(
                    "applying the display layout failed: {}",
                    display_change_reason(code)
                ),
            });
        }
        Ok(())
    }
}

pub(super) fn attached_adapters() -> Vec<Adapter> {
    let mut adapters = Vec::new();
    for index in 0u32.. {
        let mut device = display_device();
        if unsafe { EnumDisplayDevicesW(null(), index, &mut device, 0) } == 0 {
            break;
        }
        let attached = device.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP != 0;
        let mirror = device.StateFlags & DISPLAY_DEVICE_MIRRORING_DRIVER != 0;
        if !attached || mirror {
            continue;
        }
        let name = from_wide(&device.DeviceName);
        adapters.push(Adapter {
            interface: monitor_interface(&name),
            primary: device.StateFlags & DISPLAY_DEVICE_PRIMARY_DEVICE != 0,
            device: name,
        });
    }
    adapters
}

pub(super) fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

pub(super) fn from_wide(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

fn adapter_for(handle: &DisplayHandle, capability: &'static str) -> Result<Adapter, DisplayError> {
    attached_adapters()
        .into_iter()
        .find(|adapter| adapter.connector() == handle.connector())
        .ok_or_else(|| DisplayError::NotFound {
            capability,
            selector: handle.connector().to_string(),
        })
}

fn display_device() -> DISPLAY_DEVICEW {
    let mut device: DISPLAY_DEVICEW = unsafe { std::mem::zeroed() };
    device.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;
    device
}

fn devmode() -> DEVMODEW {
    let mut devmode: DEVMODEW = unsafe { std::mem::zeroed() };
    devmode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
    devmode
}

fn monitor_interface(device: &str) -> Option<String> {
    let name = wide(device);
    let mut monitor = display_device();
    let found = unsafe {
        EnumDisplayDevicesW(
            name.as_ptr(),
            0,
            &mut monitor,
            EDD_GET_DEVICE_INTERFACE_NAME,
        )
    };
    let interface = from_wide(&monitor.DeviceID);
    (found != 0 && !interface.is_empty()).then_some(interface)
}

fn read_edid(instance: &str) -> Option<Vec<u8>> {
    let key = wide(&edid_registry_key(instance));
    let value = wide("EDID");
    let mut size = 0u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_BINARY,
            null_mut(),
            null_mut(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS || size == 0 {
        return None;
    }
    let mut edid = vec![0u8; size as usize];
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_BINARY,
            null_mut(),
            edid.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    edid.truncate(size as usize);
    Some(edid)
}

fn current_settings(device: &str) -> Result<DEVMODEW, DisplayError> {
    let name = wide(device);
    let mut current = devmode();
    if unsafe { EnumDisplaySettingsExW(name.as_ptr(), ENUM_CURRENT_SETTINGS, &mut current, 0) } == 0
    {
        return Err(DisplayError::Io(std::io::Error::other(format!(
            "EnumDisplaySettingsExW cannot read the current mode of {device}"
        ))));
    }
    Ok(current)
}

fn all_settings(device: &str) -> Vec<DisplaySetting> {
    let name = wide(device);
    let mut settings = Vec::new();
    for index in 0u32.. {
        let mut mode = devmode();
        if unsafe { EnumDisplaySettingsExW(name.as_ptr(), index, &mut mode, 0) } == 0 {
            break;
        }
        settings.push(setting_of(&mode));
    }
    settings
}

fn setting_of(devmode: &DEVMODEW) -> DisplaySetting {
    DisplaySetting {
        width: devmode.dmPelsWidth,
        height: devmode.dmPelsHeight,
        refresh_hz: devmode.dmDisplayFrequency,
        bits_per_pixel: devmode.dmBitsPerPel,
    }
}

fn position_of(devmode: &DEVMODEW) -> POINTL {
    unsafe { devmode.Anonymous1.Anonymous2.dmPosition }
}

fn change(device: &str, devmode: &DEVMODEW, flags: CDS_TYPE) -> Result<(), DisplayError> {
    let name = wide(device);
    let code =
        unsafe { ChangeDisplaySettingsExW(name.as_ptr(), devmode, null_mut(), flags, null()) };
    if code == DISP_CHANGE_SUCCESSFUL {
        return Ok(());
    }
    Err(DisplayError::LayoutInvalid {
        reason: format!(
            "{} refused the display change: {}",
            connector_from_device(device),
            display_change_reason(code)
        ),
    })
}
