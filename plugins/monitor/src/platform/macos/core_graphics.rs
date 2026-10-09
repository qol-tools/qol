use qol_windowing::display::{cg_display_id_from_connector, DisplayHandle};

use crate::monitor::backends::avservice::{AvError, DisplayIdentity};
use crate::monitor::backends::cg_gamma::CgGammaSeam;
use crate::monitor::GammaTable;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGDisplayVendorNumber(display: u32) -> u32;
    fn CGDisplayModelNumber(display: u32) -> u32;
    fn CGDisplaySerialNumber(display: u32) -> u32;
    fn CGDisplayGammaTableCapacity(display: u32) -> u32;
    fn CGGetDisplayTransferByTable(
        display: u32,
        capacity: u32,
        red: *mut f32,
        green: *mut f32,
        blue: *mut f32,
        sample_count: *mut u32,
    ) -> i32;
    fn CGSetDisplayTransferByTable(
        display: u32,
        table_size: u32,
        red: *const f32,
        green: *const f32,
        blue: *const f32,
    ) -> i32;
}

pub struct CoreGraphicsSeam;

impl CgGammaSeam for CoreGraphicsSeam {
    fn read_table(&self, display_id: u32) -> Option<GammaTable> {
        let mut sample_count = unsafe { CGDisplayGammaTableCapacity(display_id) };
        if sample_count == 0 {
            return None;
        }
        let mut red = vec![0f32; sample_count as usize];
        let mut green = vec![0f32; sample_count as usize];
        let mut blue = vec![0f32; sample_count as usize];
        let error = unsafe {
            CGGetDisplayTransferByTable(
                display_id,
                sample_count,
                red.as_mut_ptr(),
                green.as_mut_ptr(),
                blue.as_mut_ptr(),
                &mut sample_count,
            )
        };
        if error != 0 {
            return None;
        }
        let to_u16 = |value: f32| (value.clamp(0.0, 1.0) * 65535.0).round() as u16;
        Some(GammaTable {
            red: red.into_iter().map(to_u16).collect(),
            green: green.into_iter().map(to_u16).collect(),
            blue: blue.into_iter().map(to_u16).collect(),
        })
    }

    fn write_table(&self, display_id: u32, table: &GammaTable) -> bool {
        let to_f32 = |entries: &[u16]| {
            entries
                .iter()
                .map(|entry| f32::from(*entry) / 65535.0)
                .collect::<Vec<f32>>()
        };
        let red = to_f32(&table.red);
        let green = to_f32(&table.green);
        let blue = to_f32(&table.blue);
        let error = unsafe {
            CGSetDisplayTransferByTable(
                display_id,
                red.len() as u32,
                red.as_ptr(),
                green.as_ptr(),
                blue.as_ptr(),
            )
        };
        error == 0
    }
}

pub(crate) fn display_identity(handle: &DisplayHandle) -> Result<Option<DisplayIdentity>, AvError> {
    let Some(display_id) = cg_display_id_from_connector(handle.connector()) else {
        return Ok(None);
    };
    Ok(Some(DisplayIdentity {
        vendor: unsafe { CGDisplayVendorNumber(display_id) },
        model: unsafe { CGDisplayModelNumber(display_id) },
        serial: unsafe { CGDisplaySerialNumber(display_id) },
    }))
}
