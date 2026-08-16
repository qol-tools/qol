use std::collections::HashMap;
use std::sync::Mutex;

use qol_windowing::DisplayEnumerator;

use crate::monitor::backends::x11_randr_gamma::MISMATCH_WARN_AT;
use crate::monitor::{
    BrightnessSource, BrightnessState, DisplayCapabilities, DisplayControl, DisplayHandle,
    DisplayMode, GammaState, GammaStateControl, GammaTable, HdrState, MonitorError, RestoreOutcome,
    HDR_REASON, MODES_REASON,
};
use crate::session::{LutProvider, LutRestoreOutcome};

const MIN_PERCENT: u8 = 10;

pub fn display_id_from_connector(connector: &str) -> Option<u32> {
    let suffix = connector.strip_prefix("cg-")?;
    let id = suffix.strip_suffix("-builtin").unwrap_or(suffix);
    id.parse().ok()
}

fn scaled_table(original: &GammaTable, percent: u8) -> GammaTable {
    let factor = u32::from(percent.clamp(MIN_PERCENT, 100));
    let scale = |entry: u16| (u32::from(entry) * factor / 100) as u16;
    GammaTable {
        red: original.red.iter().map(|entry| scale(*entry)).collect(),
        green: original.green.iter().map(|entry| scale(*entry)).collect(),
        blue: original.blue.iter().map(|entry| scale(*entry)).collect(),
    }
}

pub trait CgGammaSeam: Send + Sync {
    fn read_table(&self, display_id: u32) -> Option<GammaTable>;
    fn write_table(&self, display_id: u32, table: &GammaTable) -> bool;
}

#[derive(Default)]
struct GammaSession {
    original: Option<GammaTable>,
    written_checksum: Option<u64>,
    written_value: Option<u8>,
    mismatches: usize,
    warned: bool,
}

pub struct CgGammaControl<T: CgGammaSeam> {
    seam: T,
    sessions: Mutex<HashMap<String, GammaSession>>,
}

impl<T: CgGammaSeam> CgGammaControl<T> {
    pub fn new(seam: T) -> Self {
        Self {
            seam,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn session(&self) -> std::sync::MutexGuard<'_, HashMap<String, GammaSession>> {
        self.sessions.lock().unwrap()
    }

    fn display_id(&self, handle: &DisplayHandle) -> Result<u32, MonitorError> {
        display_id_from_connector(handle.connector()).ok_or_else(|| {
            MonitorError::unsupported(
                "brightness",
                format!("no CG display id parses from {}", handle.connector()),
            )
        })
    }

    fn get_inner(&self, handle: &DisplayHandle) -> Result<u8, MonitorError> {
        Ok(self
            .session()
            .get(handle.id())
            .and_then(|entry| entry.written_value)
            .unwrap_or(100))
    }

    fn set_inner(&self, handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
        let display_id = self.display_id(handle)?;
        let current = self.seam.read_table(display_id).ok_or_else(|| {
            MonitorError::unsupported(
                "brightness",
                format!("no gamma table is readable for display {display_id}"),
            )
        })?;
        if current.size() < 2 {
            return Err(MonitorError::unsupported(
                "brightness",
                format!(
                    "the gamma ramp on display {display_id} is inert (size {})",
                    current.size()
                ),
            ));
        }
        let mut session = self.session();
        let entry = session.entry(handle.id().to_string()).or_default();
        let original = entry.original.get_or_insert_with(|| current.clone());
        let target = scaled_table(original, value);
        let mut verified = false;
        for _ in 0..=1 {
            if !self.seam.write_table(display_id, &target) {
                break;
            }
            let Some(back) = self.seam.read_table(display_id) else {
                break;
            };
            if back.size() == target.size() && back.checksum() == target.checksum() {
                verified = true;
                break;
            }
            if back.size() != target.size() {
                break;
            }
        }
        if verified {
            entry.written_checksum = Some(target.checksum());
            entry.written_value = Some(value.clamp(MIN_PERCENT, 100));
        } else {
            entry.mismatches += 1;
            if entry.mismatches >= MISMATCH_WARN_AT {
                entry.warned = true;
            }
        }
        Ok(())
    }

    fn restore_lut(
        &self,
        handle: &DisplayHandle,
        original: &GammaTable,
        last_value: u8,
    ) -> Result<RestoreOutcome, MonitorError> {
        let guard = {
            let session = self.session();
            match session.get(handle.id()) {
                Some(entry) => entry
                    .written_checksum
                    .unwrap_or_else(|| scaled_table(original, last_value).checksum()),
                None => scaled_table(original, last_value).checksum(),
            }
        };
        let display_id = self.display_id(handle)?;
        let current = self.seam.read_table(display_id).ok_or_else(|| {
            MonitorError::unsupported(
                "gamma",
                format!("no gamma table is readable for display {display_id}"),
            )
        })?;
        if current.checksum() != guard {
            return Ok(RestoreOutcome::ForeignLutPreserved);
        }
        if !self.seam.write_table(display_id, original) {
            return Err(MonitorError::unsupported(
                "gamma",
                "the restore write failed on the CoreGraphics seam",
            ));
        }
        let back = self.seam.read_table(display_id).ok_or_else(|| {
            MonitorError::unsupported(
                "gamma",
                format!("no gamma table is readable for display {display_id}"),
            )
        })?;
        if back.size() != original.size() || back.checksum() != original.checksum() {
            let mut session = self.session();
            let entry = session.entry(handle.id().to_string()).or_default();
            entry.mismatches += 1;
            if entry.mismatches >= MISMATCH_WARN_AT {
                entry.warned = true;
            }
            return Err(MonitorError::unsupported(
                "gamma",
                "the restore write did not verify; a co-owner changed the LUT during restore",
            ));
        }
        let mut session = self.session();
        if let Some(entry) = session.get_mut(handle.id()) {
            entry.original = None;
            entry.written_checksum = None;
            entry.written_value = None;
        }
        Ok(RestoreOutcome::Restored)
    }

    fn restore_inner(&self, handle: &DisplayHandle) -> Result<RestoreOutcome, MonitorError> {
        let session = self.session();
        let Some(entry) = session.get(handle.id()) else {
            return Ok(RestoreOutcome::NothingToRestore);
        };
        let (Some(original), Some(_)) = (&entry.original, entry.written_checksum) else {
            return Ok(RestoreOutcome::NothingToRestore);
        };
        let last_value = entry.written_value.unwrap_or(100);
        let original = original.clone();
        drop(session);
        self.restore_lut(handle, &original, last_value)
    }
}

impl<T: CgGammaSeam> DisplayControl for CgGammaControl<T> {
    fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError> {
        Ok(qol_windowing::Platform.enumerate()?)
    }

    fn probe(&self, handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError> {
        let Some(display_id) = display_id_from_connector(handle.connector()) else {
            return Ok(DisplayCapabilities::none());
        };
        let size = self
            .seam
            .read_table(display_id)
            .map(|table| table.size())
            .unwrap_or(0);
        Ok(DisplayCapabilities {
            brightness_gamma: size >= 2,
            ..DisplayCapabilities::none()
        })
    }

    fn get_brightness(&self, handle: &DisplayHandle) -> Result<BrightnessState, MonitorError> {
        let value = self.get_inner(handle)?;
        Ok(BrightnessState {
            value,
            source: BrightnessSource::Gamma,
        })
    }

    fn set_brightness(&self, handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
        self.set_inner(handle, value)
    }

    fn get_gamma(&self, handle: &DisplayHandle) -> Result<GammaState, MonitorError> {
        let value = self.get_inner(handle)?;
        Ok(GammaState { value })
    }

    fn set_gamma(&self, handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
        self.set_inner(handle, value)
    }

    fn list_modes(&self, _handle: &DisplayHandle) -> Result<Vec<DisplayMode>, MonitorError> {
        Err(MonitorError::unsupported("modes", MODES_REASON))
    }

    fn set_mode(&self, _handle: &DisplayHandle, _mode: &DisplayMode) -> Result<(), MonitorError> {
        Err(MonitorError::unsupported("modes", MODES_REASON))
    }

    fn get_hdr(&self, _handle: &DisplayHandle) -> Result<HdrState, MonitorError> {
        Err(MonitorError::unsupported("hdr", HDR_REASON))
    }

    fn set_hdr(&self, _handle: &DisplayHandle, _enabled: bool) -> Result<(), MonitorError> {
        Err(MonitorError::unsupported("hdr", HDR_REASON))
    }
}

impl<T: CgGammaSeam> GammaStateControl for CgGammaControl<T> {
    fn mismatch_count(&self, handle: &DisplayHandle) -> usize {
        self.session()
            .get(handle.id())
            .map(|entry| entry.mismatches)
            .unwrap_or(0)
    }

    fn warned(&self, handle: &DisplayHandle) -> bool {
        self.session()
            .get(handle.id())
            .map(|entry| entry.warned)
            .unwrap_or(false)
    }

    fn restore(&self, handle: &DisplayHandle) -> Result<RestoreOutcome, MonitorError> {
        self.restore_inner(handle)
    }
}

impl<T: CgGammaSeam> LutProvider for CgGammaControl<T> {
    fn capture(&self, connector: &str) -> Option<GammaTable> {
        let display_id = display_id_from_connector(connector)?;
        self.seam.read_table(display_id)
    }

    fn write_guarded(
        &self,
        handle: &DisplayHandle,
        original: &GammaTable,
        last_value: u8,
    ) -> LutRestoreOutcome {
        match self.restore_lut(handle, original, last_value) {
            Ok(RestoreOutcome::Restored) => LutRestoreOutcome::Restored,
            Ok(RestoreOutcome::ForeignLutPreserved) => LutRestoreOutcome::ForeignLutPreserved,
            Ok(RestoreOutcome::NothingToRestore) | Err(_) => LutRestoreOutcome::Unavailable,
        }
    }
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
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

#[cfg(target_os = "macos")]
pub struct CoreGraphicsSeam;

#[cfg(target_os = "macos")]
impl CgGammaSeam for CoreGraphicsSeam {
    fn read_table(&self, display_id: u32) -> Option<GammaTable> {
        let mut sample_count = 0u32;
        let error = unsafe {
            CGGetDisplayTransferByTable(
                display_id,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut sample_count,
            )
        };
        if error != 0 || sample_count == 0 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct FakeCgSeam {
        tables: Arc<Mutex<HashMap<u32, GammaTable>>>,
    }

    impl CgGammaSeam for FakeCgSeam {
        fn read_table(&self, display_id: u32) -> Option<GammaTable> {
            self.tables.lock().unwrap().get(&display_id).cloned()
        }

        fn write_table(&self, display_id: u32, table: &GammaTable) -> bool {
            self.tables
                .lock()
                .unwrap()
                .insert(display_id, table.clone());
            true
        }
    }

    fn identity_table(size: usize, base: u16) -> GammaTable {
        GammaTable {
            red: (0..size).map(|i| base + i as u16).collect(),
            green: (0..size).map(|i| base + 2 * i as u16).collect(),
            blue: (0..size).map(|i| base + 3 * i as u16).collect(),
        }
    }

    fn handle(connector: &str) -> DisplayHandle {
        DisplayHandle::new(format!("mac-{connector}"), connector.into(), None, false)
    }

    fn backend(seam: FakeCgSeam) -> CgGammaControl<FakeCgSeam> {
        CgGammaControl::new(seam)
    }

    fn seam_with(table: GammaTable) -> FakeCgSeam {
        FakeCgSeam {
            tables: Arc::new(Mutex::new(HashMap::from([(1, table)]))),
        }
    }

    #[test]
    fn scaled_table_math_scales_every_channel() {
        let table = identity_table(4, 100);
        assert_eq!(
            scaled_table(&table, 50),
            GammaTable {
                red: vec![50, 50, 51, 51],
                green: vec![50, 51, 52, 53],
                blue: vec![50, 51, 53, 54],
            }
        );
        assert_eq!(scaled_table(&table, 100), table);
    }

    #[test]
    fn scaled_table_clamps_to_the_ten_percent_floor() {
        let table = identity_table(4, 1000);
        let floor = scaled_table(&table, 10);
        assert_eq!(floor.red, vec![100; 4]);
        assert_eq!(scaled_table(&table, 5), floor);
        assert_eq!(scaled_table(&table, 0), floor);
        assert_eq!(scaled_table(&table, 100), table);
    }

    #[test]
    fn set_then_restore_round_trips_the_pristine_table() {
        let original = identity_table(4, 100);
        let backend = backend(seam_with(original.clone()));
        let display = handle("cg-1");
        backend.set_brightness(&display, 40).unwrap();
        let state = backend.get_brightness(&display).unwrap();
        assert_eq!(state.value, 40);
        assert_eq!(state.source, BrightnessSource::Gamma);
        {
            let tables = backend.seam.tables.lock().unwrap();
            assert_eq!(tables[&1], scaled_table(&original, 40));
        }
        assert_eq!(backend.restore(&display).unwrap(), RestoreOutcome::Restored);
        let tables = backend.seam.tables.lock().unwrap();
        assert_eq!(tables[&1], original);
        drop(tables);
        assert_eq!(
            backend.restore(&display).unwrap(),
            RestoreOutcome::NothingToRestore,
            "restore is idempotent"
        );
    }

    #[test]
    fn get_before_any_set_reports_full_brightness() {
        let backend = backend(seam_with(identity_table(4, 100)));
        let state = backend.get_brightness(&handle("cg-1")).unwrap();
        assert_eq!(state.value, 100);
    }

    #[test]
    fn set_below_the_floor_writes_the_floor_and_get_returns_it() {
        let original = identity_table(4, 1000);
        let backend = backend(seam_with(original.clone()));
        let display = handle("cg-1");
        backend.set_brightness(&display, 3).unwrap();
        assert_eq!(backend.get_brightness(&display).unwrap().value, 10);
        let tables = backend.seam.tables.lock().unwrap();
        assert_eq!(tables[&1], scaled_table(&original, 10));
    }

    #[test]
    fn probe_reports_gamma_when_a_real_ramp_is_readable() {
        let backend = backend(seam_with(identity_table(4, 100)));
        let caps = backend.probe(&handle("cg-1")).unwrap();
        assert_eq!(
            caps,
            DisplayCapabilities {
                brightness_gamma: true,
                ..DisplayCapabilities::none()
            }
        );
    }

    #[test]
    fn write_guarded_restores_the_original_without_a_live_session() {
        let original = identity_table(4, 100);
        let backend = backend(seam_with(scaled_table(&original, 50)));
        let outcome = backend.write_guarded(&handle("cg-1"), &original, 50);
        assert_eq!(outcome, LutRestoreOutcome::Restored);
        let tables = backend.seam.tables.lock().unwrap();
        assert_eq!(tables[&1], original);
    }

    #[test]
    fn write_guarded_preserves_a_foreign_lut() {
        let original = identity_table(4, 100);
        let mut foreign = scaled_table(&original, 50);
        foreign.red[0] += 1;
        let backend = backend(seam_with(foreign.clone()));
        let outcome = backend.write_guarded(&handle("cg-1"), &original, 50);
        assert_eq!(outcome, LutRestoreOutcome::ForeignLutPreserved);
        let tables = backend.seam.tables.lock().unwrap();
        assert_eq!(tables[&1], foreign);
    }

    #[test]
    fn display_id_from_connector_parses_cg_connectors() {
        assert_eq!(display_id_from_connector("cg-123"), Some(123));
        assert_eq!(display_id_from_connector("cg-7-builtin"), Some(7));
        assert_eq!(display_id_from_connector("card0-DP-1"), None);
    }
}
