use std::ptr::{null, null_mut};

use windows_sys::Win32::Devices::Display::{
    DestroyPhysicalMonitors, GetNumberOfPhysicalMonitorsFromHMONITOR,
    GetPhysicalMonitorsFromHMONITOR, GetVCPFeatureAndVCPFeatureReply, SetVCPFeature,
    PHYSICAL_MONITOR,
};
use windows_sys::Win32::Foundation::{BOOL, LPARAM, RECT, TRUE};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};

use super::display::from_wide;
use crate::monitor::backends::gdi_display::connector_from_device;
use crate::monitor::backends::vcp_ddc::{VcpReading, VcpTransport};
use crate::monitor::I2cError;

pub(super) struct Dxva2Transport;

struct PhysicalMonitors(Vec<PHYSICAL_MONITOR>);

impl Drop for PhysicalMonitors {
    fn drop(&mut self) {
        unsafe {
            DestroyPhysicalMonitors(self.0.len() as u32, self.0.as_ptr());
        }
    }
}

impl VcpTransport for Dxva2Transport {
    fn read(&self, connector: &str, code: u8) -> Result<VcpReading, I2cError> {
        let monitors = physical_monitors(connector)?;
        let handle = monitors.0[0].hPhysicalMonitor;
        let mut current = 0u32;
        let mut max = 0u32;
        let answered = unsafe {
            GetVCPFeatureAndVCPFeatureReply(handle, code, null_mut(), &mut current, &mut max)
        };
        if answered == 0 {
            return Err(vcp_failed(connector, "read", code));
        }
        Ok(VcpReading { current, max })
    }

    fn write(&self, connector: &str, code: u8, value: u32) -> Result<(), I2cError> {
        let monitors = physical_monitors(connector)?;
        for monitor in &monitors.0 {
            let handle = monitor.hPhysicalMonitor;
            if unsafe { SetVCPFeature(handle, code, value) } == 0 {
                return Err(vcp_failed(connector, "write", code));
            }
        }
        Ok(())
    }
}

fn physical_monitors(connector: &str) -> Result<PhysicalMonitors, I2cError> {
    let hmonitor = monitor_for(connector)?;
    let mut count = 0u32;
    if unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor, &mut count) } == 0 || count == 0 {
        return Err(I2cError::UnsupportedTransport {
            detail: format!(
                "{connector} exposes no DDC/CI physical monitor: {}",
                std::io::Error::last_os_error()
            ),
        });
    }
    let mut monitors = vec![unsafe { std::mem::zeroed::<PHYSICAL_MONITOR>() }; count as usize];
    if unsafe { GetPhysicalMonitorsFromHMONITOR(hmonitor, count, monitors.as_mut_ptr()) } == 0 {
        return Err(I2cError::UnsupportedTransport {
            detail: format!(
                "{connector} physical monitors are unavailable: {}",
                std::io::Error::last_os_error()
            ),
        });
    }
    Ok(PhysicalMonitors(monitors))
}

fn monitor_for(connector: &str) -> Result<HMONITOR, I2cError> {
    let mut monitors: Vec<HMONITOR> = Vec::new();
    unsafe {
        EnumDisplayMonitors(
            null_mut(),
            null(),
            Some(collect_monitor),
            &mut monitors as *mut Vec<HMONITOR> as LPARAM,
        );
    }
    monitors
        .into_iter()
        .find(|monitor| {
            device_of(*monitor).is_some_and(|device| connector_from_device(&device) == connector)
        })
        .ok_or_else(|| I2cError::UnsupportedTransport {
            detail: format!("no active monitor matches {connector}"),
        })
}

fn device_of(monitor: HMONITOR) -> Option<String> {
    let mut info: MONITORINFOEXW = unsafe { std::mem::zeroed() };
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    let read = unsafe {
        GetMonitorInfoW(
            monitor,
            &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
        )
    };
    (read != 0).then(|| from_wide(&info.szDevice))
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let monitors = unsafe { &mut *(data as *mut Vec<HMONITOR>) };
    monitors.push(monitor);
    TRUE
}

fn vcp_failed(connector: &str, operation: &str, code: u8) -> I2cError {
    I2cError::UnsupportedTransport {
        detail: format!(
            "{connector} did not answer the DDC/CI VCP {code:#04x} {operation}: {}",
            std::io::Error::last_os_error()
        ),
    }
}
