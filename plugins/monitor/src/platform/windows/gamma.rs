use std::ptr::null;
use std::sync::OnceLock;

use qol_platform::native::registry::{self, Hive};
use qol_platform::native::wide::wide_nul;
use windows_sys::Win32::Graphics::Gdi::{CreateDCW, DeleteDC, HDC};
use windows_sys::Win32::UI::ColorSystem::{GetDeviceGammaRamp, SetDeviceGammaRamp};

use super::display::attached_adapters;
use crate::monitor::backends::gdi_display::connector_from_device;
use crate::monitor::backends::gdi_gamma::{
    self, ramp_from_table, table_from_ramp, GammaRamp, DEFAULT_GAMMA_RANGE, RAMP_SIZE,
};
use crate::monitor::backends::x11_randr_gamma::{GammaBus, GammaTransport};
use crate::monitor::night::Tint;
use crate::monitor::{GammaError, GammaTable};

const ICM_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ICM";
const GAMMA_RANGE_VALUE: &str = "GdiIcmGammaRange";
const WIDEST_GAMMA_RANGE: u32 = 256;

pub(super) struct GdiGammaTransport;

pub(crate) fn fit_tint(dimmed: &GammaTable, tint: Tint) -> Tint {
    gdi_gamma::fit_tint(dimmed, tint, gamma_range())
}

fn gamma_range() -> u16 {
    static RANGE: OnceLock<u16> = OnceLock::new();
    *RANGE.get_or_init(|| {
        registry::read_dword(Hive::LocalMachine, ICM_KEY, GAMMA_RANGE_VALUE)
            .ok()
            .flatten()
            .map_or(DEFAULT_GAMMA_RANGE, |range| {
                range.min(WIDEST_GAMMA_RANGE) as u16
            })
    })
}

pub(super) struct GdiGammaBus {
    devices: Vec<String>,
}

struct DeviceContext(HDC);

impl Drop for DeviceContext {
    fn drop(&mut self) {
        unsafe {
            DeleteDC(self.0);
        }
    }
}

impl GammaTransport for GdiGammaTransport {
    type Bus = GdiGammaBus;

    fn open(&self) -> Result<Self::Bus, GammaError> {
        Ok(GdiGammaBus {
            devices: attached_adapters()
                .into_iter()
                .map(|adapter| adapter.device)
                .collect(),
        })
    }
}

impl GdiGammaBus {
    fn device(&self, crtc: u32) -> Result<&str, GammaError> {
        self.devices
            .get(crtc as usize)
            .map(String::as_str)
            .ok_or_else(|| GammaError::Unsupported {
                detail: format!("display {crtc} left the desktop"),
            })
    }

    fn context(&self, crtc: u32) -> Result<(DeviceContext, &str), GammaError> {
        let device = self.device(crtc)?;
        let driver = wide_nul("DISPLAY");
        let name = wide_nul(device);
        let hdc = unsafe { CreateDCW(driver.as_ptr(), name.as_ptr(), null(), null()) };
        if hdc.is_null() {
            return Err(GammaError::Unsupported {
                detail: format!(
                    "cannot open a device context on {}: {}",
                    connector_from_device(device),
                    std::io::Error::last_os_error()
                ),
            });
        }
        Ok((DeviceContext(hdc), device))
    }
}

impl GammaBus for GdiGammaBus {
    fn crtc_for_connector(&mut self, connector: &str) -> Result<Option<u32>, GammaError> {
        Ok(self
            .devices
            .iter()
            .position(|device| connector_from_device(device) == connector)
            .map(|index| index as u32))
    }

    fn read_gamma(&mut self, crtc: u32) -> Result<GammaTable, GammaError> {
        let (context, device) = self.context(crtc)?;
        let mut ramp: GammaRamp = [[0u16; RAMP_SIZE]; 3];
        if unsafe { GetDeviceGammaRamp(context.0, ramp.as_mut_ptr().cast()) } == 0 {
            return Err(GammaError::Unsupported {
                detail: format!(
                    "GetDeviceGammaRamp failed on {}: {}",
                    connector_from_device(device),
                    std::io::Error::last_os_error()
                ),
            });
        }
        Ok(table_from_ramp(&ramp))
    }

    fn write_gamma(&mut self, crtc: u32, table: &GammaTable) -> Result<(), GammaError> {
        let ramp = ramp_from_table(table)?;
        let (context, device) = self.context(crtc)?;
        if unsafe { SetDeviceGammaRamp(context.0, ramp.as_ptr().cast()) } == 0 {
            return Err(GammaError::Refused {
                reason: format!(
                    "Windows refused the gamma ramp on {}; GDI rejects ramps far from identity",
                    connector_from_device(device)
                ),
            });
        }
        Ok(())
    }

    fn hdr_active(&mut self, _crtc: u32) -> Result<bool, GammaError> {
        Err(GammaError::Unsupported {
            detail: "HDR state is not readable through the GDI gamma ramp".into(),
        })
    }
}
