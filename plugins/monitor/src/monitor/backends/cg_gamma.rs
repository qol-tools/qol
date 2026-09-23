use std::collections::HashMap;
use std::sync::Mutex;

use qol_windowing::display::cg_display_id_from_connector;
use qol_windowing::DisplayEnumerator;

use crate::display_color;
use crate::display_color::classifier;
use crate::monitor::backends::x11_randr_gamma::MISMATCH_WARN_AT;
use crate::monitor::night::Tint;
use crate::monitor::{
    BrightnessSource, BrightnessState, DisplayCapabilities, DisplayControl, DisplayHandle,
    DisplayMode, GammaState, GammaStateControl, GammaTable, HdrState, MonitorError, RestoreOutcome,
    GAMMA_CHANGED_UNDER_WRITE_REASON, HDR_REASON, MODES_REASON,
};
use crate::session::{LutProvider, LutRestoreOutcome};

const MIN_PERCENT: u8 = 10;

#[cfg(test)]
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

struct GammaSession {
    original: Option<GammaTable>,
    compose_base: Option<GammaTable>,
    foreign_base: bool,
    tint_allowed: bool,
    written_checksum: Option<u64>,
    written_value: Option<u8>,
    written_tint: Tint,
    mismatches: usize,
    warned: bool,
}

impl Default for GammaSession {
    fn default() -> Self {
        Self {
            original: None,
            compose_base: None,
            foreign_base: false,
            tint_allowed: true,
            written_checksum: None,
            written_value: None,
            written_tint: Tint::NEUTRAL,
            mismatches: 0,
            warned: false,
        }
    }
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
        cg_display_id_from_connector(handle.connector()).ok_or_else(|| {
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

    fn set_inner(
        &self,
        handle: &DisplayHandle,
        value: Option<u8>,
        tint: Option<Tint>,
        expected: Option<u64>,
        capability: &'static str,
    ) -> Result<(), MonitorError> {
        let display_id = self.display_id(handle)?;
        let current = self.seam.read_table(display_id).ok_or_else(|| {
            MonitorError::unsupported(
                "brightness",
                format!("no gamma table is readable for display {display_id}"),
            )
        })?;
        if expected.is_some_and(|expected| current.checksum() != expected) {
            return Err(MonitorError::refused(
                "gamma",
                GAMMA_CHANGED_UNDER_WRITE_REASON,
            ));
        }
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
        if entry.original.is_none() {
            let (base, verdict) = display_color::compose_base(&current);
            entry.original = Some(current.clone());
            entry.compose_base = Some(base);
            entry.foreign_base = verdict.base == classifier::BaseChoice::Neutral;
            entry.tint_allowed = verdict.tint_allowed;
        }
        let base = entry
            .compose_base
            .clone()
            .unwrap_or_else(|| current.clone());
        let value = value
            .unwrap_or(entry.written_value.unwrap_or(100))
            .clamp(MIN_PERCENT, 100);
        let tint = tint.unwrap_or(entry.written_tint);
        if !entry.tint_allowed && !tint.is_neutral() {
            return Err(MonitorError::refused(
                capability,
                format!(
                    "the gamma ramp on {} carries foreign warmth; night tint is not stacked on another display owner",
                    handle.connector()
                ),
            ));
        }
        let original = entry.original.clone().unwrap_or_else(|| current.clone());
        let target =
            display_color::composed_target(&original, &base, entry.foreign_base, value, tint);
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
            entry.written_value = Some(value);
            entry.written_tint = tint;
        } else {
            entry.mismatches += 1;
            if entry.mismatches >= MISMATCH_WARN_AT {
                entry.warned = true;
            }
            return Err(MonitorError::refused(
                "gamma",
                "the CoreGraphics gamma write did not verify",
            ));
        }
        Ok(())
    }

    fn restore_lut(
        &self,
        handle: &DisplayHandle,
        original: &GammaTable,
        last_value: u8,
        last_tint: Tint,
    ) -> Result<RestoreOutcome, MonitorError> {
        let (guard_base, guard_verdict) = display_color::compose_base(original);
        let guard_foreign = guard_verdict.base == classifier::BaseChoice::Neutral;
        let entry = self.session().get(handle.id()).map(|entry| {
            (
                entry.written_checksum,
                entry.compose_base.clone(),
                entry.foreign_base,
            )
        });
        let stored = entry.as_ref().and_then(|(stored, _, _)| *stored);
        let entry_base = entry.as_ref().and_then(|(_, base, _)| base.clone());
        let entry_foreign = entry
            .as_ref()
            .map(|(_, _, foreign)| *foreign)
            .unwrap_or(guard_foreign);
        let display_id = self.display_id(handle)?;
        let current = self.seam.read_table(display_id).ok_or_else(|| {
            MonitorError::unsupported(
                "gamma",
                format!("no gamma table is readable for display {display_id}"),
            )
        })?;
        let accepted = display_color::guard_accepts(
            &current,
            original,
            &guard_base,
            guard_foreign,
            last_value,
            last_tint,
            stored,
        ) || entry_base.as_ref().is_some_and(|base| {
            display_color::guard_accepts(
                &current,
                original,
                base,
                entry_foreign,
                last_value,
                last_tint,
                stored,
            )
        });
        if !accepted {
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
            entry.compose_base = None;
            entry.foreign_base = false;
            entry.tint_allowed = true;
            entry.written_checksum = None;
            entry.written_value = None;
            entry.written_tint = Tint::NEUTRAL;
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
        let last_tint = entry.written_tint;
        let original = original.clone();
        drop(session);
        self.restore_lut(handle, &original, last_value, last_tint)
    }
}

impl<T: CgGammaSeam> DisplayControl for CgGammaControl<T> {
    fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError> {
        Ok(qol_windowing::Platform.enumerate()?)
    }

    fn probe(&self, handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError> {
        let Some(display_id) = cg_display_id_from_connector(handle.connector()) else {
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
        self.set_inner(handle, Some(value), None, None, "brightness")
    }

    fn set_brightness_with_tint(
        &self,
        handle: &DisplayHandle,
        value: u8,
        tint: Tint,
    ) -> Result<(), MonitorError> {
        self.set_gamma_adjustment(handle, value, tint)
    }

    fn set_tint(&self, handle: &DisplayHandle, tint: Tint) -> Result<(), MonitorError> {
        self.set_inner(handle, None, Some(tint), None, "tint")
    }

    fn set_gamma_adjustment(
        &self,
        handle: &DisplayHandle,
        value: u8,
        tint: Tint,
    ) -> Result<(), MonitorError> {
        self.set_inner(handle, Some(value), Some(tint), None, "gamma")
    }

    fn set_gamma_adjustment_guarded(
        &self,
        handle: &DisplayHandle,
        value: u8,
        tint: Tint,
        expected: u64,
    ) -> Result<(), MonitorError> {
        self.set_inner(handle, Some(value), Some(tint), Some(expected), "gamma")
    }

    fn get_gamma(&self, handle: &DisplayHandle) -> Result<GammaState, MonitorError> {
        let value = self.get_inner(handle)?;
        Ok(GammaState { value })
    }

    fn set_gamma(&self, handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
        self.set_inner(handle, Some(value), None, None, "gamma")
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
        let display_id = cg_display_id_from_connector(connector)?;
        self.seam.read_table(display_id)
    }

    fn write_guarded(
        &self,
        handle: &DisplayHandle,
        original: &GammaTable,
        last_value: u8,
        last_tint: Tint,
    ) -> LutRestoreOutcome {
        match self.restore_lut(handle, original, last_value, last_tint) {
            Ok(RestoreOutcome::Restored) => LutRestoreOutcome::Restored,
            Ok(RestoreOutcome::ForeignLutPreserved) => LutRestoreOutcome::ForeignLutPreserved,
            Ok(RestoreOutcome::NothingToRestore) | Err(_) => LutRestoreOutcome::Unavailable,
        }
    }

    fn adopt_baseline(
        &self,
        handle: &DisplayHandle,
        original: &GammaTable,
        last_value: u8,
        last_tint: Tint,
    ) {
        let mut session = self.session();
        let entry = session.entry(handle.id().to_string()).or_default();
        if entry.original.is_none() {
            let (base, verdict) = display_color::compose_base(original);
            let foreign_base = verdict.base == classifier::BaseChoice::Neutral;
            let target = display_color::composed_target(
                original,
                &base,
                foreign_base,
                last_value,
                last_tint,
            );
            entry.written_checksum = Some(target.checksum());
            entry.original = Some(original.clone());
            entry.compose_base = Some(base);
            entry.foreign_base = foreign_base;
            entry.tint_allowed = verdict.tint_allowed;
            entry.written_value = Some(last_value);
            entry.written_tint = last_tint;
        }
    }

    fn rebind_compose_base(
        &self,
        handle: &DisplayHandle,
        base: &GammaTable,
        foreign: bool,
        tint_allowed: bool,
    ) -> bool {
        let mut session = self.session();
        let Some(entry) = session.get_mut(handle.id()) else {
            return false;
        };
        entry.compose_base = Some(base.clone());
        entry.foreign_base = foreign;
        entry.tint_allowed = tint_allowed;
        true
    }
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
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

#[cfg(target_os = "macos")]
pub struct CoreGraphicsSeam;

#[cfg(target_os = "macos")]
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

    fn warm_scale_table(size: usize) -> GammaTable {
        let channel = |peak: f64| {
            (0..size)
                .map(|index| {
                    let ratio = index as f64 / (size - 1) as f64;
                    (peak * ratio.powf(1.001)).round() as u16
                })
                .collect::<Vec<u16>>()
        };
        GammaTable {
            red: channel(65535.0),
            green: channel(51110.0),
            blue: channel(35808.0),
        }
    }

    fn warm_calibration_table(size: usize) -> GammaTable {
        let channel = |bias: f64| {
            (0..size)
                .map(|index| {
                    let x = index as f64 / (size - 1) as f64;
                    let shaped = x * x * (3.0 - 2.0 * x);
                    (65535.0 * shaped * bias).min(65535.0).round() as u16
                })
                .collect::<Vec<u16>>()
        };
        GammaTable {
            red: channel(1.0),
            green: channel(0.98),
            blue: channel(0.72),
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
    fn night_tint_and_brightness_compose_and_restore_without_losing_either() {
        for percent in [10, 50, 100] {
            let original = identity_table(4, 30_000);
            let backend = backend(seam_with(original.clone()));
            let display = handle("cg-1");
            let tint = Tint::from_kelvin(3500);
            backend.set_brightness(&display, percent).unwrap();
            backend.set_tint(&display, tint).unwrap();
            assert_eq!(
                backend.seam.tables.lock().unwrap()[&1],
                original.dimmed(percent).tinted(tint),
                "percent: {percent}"
            );
            backend.set_brightness(&display, 70).unwrap();
            assert_eq!(
                backend.seam.tables.lock().unwrap()[&1],
                original.dimmed(70).tinted(tint),
                "percent: {percent}"
            );
            backend.set_tint(&display, Tint::NEUTRAL).unwrap();
            assert_eq!(
                backend.seam.tables.lock().unwrap()[&1],
                original.dimmed(70),
                "percent: {percent}"
            );
            assert_eq!(backend.restore(&display).unwrap(), RestoreOutcome::Restored);
            assert_eq!(
                backend.seam.tables.lock().unwrap()[&1],
                original,
                "percent: {percent}"
            );
        }
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
        let outcome = backend.write_guarded(&handle("cg-1"), &original, 50, Tint::NEUTRAL);
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
        let outcome = backend.write_guarded(&handle("cg-1"), &original, 50, Tint::NEUTRAL);
        assert_eq!(outcome, LutRestoreOutcome::ForeignLutPreserved);
        let tables = backend.seam.tables.lock().unwrap();
        assert_eq!(tables[&1], foreign);
    }

    #[test]
    fn a_warm_foreign_ramp_is_never_the_compose_base() {
        let foreign = warm_scale_table(64);
        let backend = backend(seam_with(foreign.clone()));
        let display = handle("cg-1");
        backend.set_tint(&display, Tint::from_kelvin(3500)).unwrap();
        let written = backend.seam.tables.lock().unwrap()[&1].clone();
        assert_eq!(u32::from(written.red[written.red.len() - 1]), 65535u32);
        assert_eq!(
            u32::from(written.green[written.green.len() - 1]),
            65535u32 * 758 / 1000
        );
        assert_eq!(
            u32::from(written.blue[written.blue.len() - 1]),
            65535u32 * 563 / 1000
        );
        let session = backend.sessions.lock().unwrap();
        let entry = session.get("mac-cg-1").unwrap();
        assert_eq!(entry.original.as_ref().unwrap(), &foreign);
        assert_ne!(entry.compose_base.as_ref().unwrap(), &foreign);
        assert!(entry.tint_allowed);
    }

    #[test]
    fn a_neutral_tint_on_a_foreign_base_returns_the_as_found_table() {
        let foreign = warm_scale_table(64);
        let backend = backend(seam_with(foreign.clone()));
        let display = handle("cg-1");
        backend.set_tint(&display, Tint::from_kelvin(3500)).unwrap();
        assert_ne!(backend.seam.tables.lock().unwrap()[&1], foreign);
        backend.set_tint(&display, Tint::NEUTRAL).unwrap();
        assert_eq!(backend.seam.tables.lock().unwrap()[&1], foreign);
    }

    #[test]
    fn a_warm_calibration_ramp_refuses_the_night_tint_without_writing() {
        let calibration = warm_calibration_table(64);
        let backend = backend(seam_with(calibration.clone()));
        let display = handle("cg-1");
        match backend
            .set_tint(&display, Tint::from_kelvin(3500))
            .unwrap_err()
        {
            MonitorError::Refused { capability, .. } => assert_eq!(capability, "tint"),
            other => panic!("expected Refused, got {other:?}"),
        }
        assert_eq!(backend.seam.tables.lock().unwrap()[&1], calibration);
        let session = backend.sessions.lock().unwrap();
        assert!(!session.get("mac-cg-1").unwrap().tint_allowed);
    }

    #[test]
    fn a_restart_recognises_its_own_neutral_base_composition_for_restore() {
        let foreign = warm_scale_table(64);
        let shared = Arc::new(Mutex::new(HashMap::from([(1, foreign.clone())])));
        let display = handle("cg-1");
        let first = backend(FakeCgSeam {
            tables: Arc::clone(&shared),
        });
        first.set_brightness(&display, 50).unwrap();
        drop(first);
        let second = backend(FakeCgSeam {
            tables: Arc::clone(&shared),
        });
        assert_eq!(
            second.write_guarded(&display, &foreign, 50, Tint::NEUTRAL),
            LutRestoreOutcome::Restored
        );
        assert_eq!(shared.lock().unwrap()[&1], foreign);
    }

    #[test]
    fn the_guard_accepts_a_pre_branch_composition() {
        let original = warm_scale_table(64);
        let warm = Tint::from_kelvin(3500);
        let current = original.dimmed(50).tinted(warm);
        let (derived_base, verdict) = display_color::compose_base(&original);
        assert_eq!(verdict.base, classifier::BaseChoice::Neutral);
        let composed = display_color::composed_target(&original, &derived_base, true, 50, warm);
        assert_ne!(current.checksum(), composed.checksum());
        let backend = backend(seam_with(current));
        let display = handle("cg-1");
        assert_eq!(
            backend.write_guarded(&display, &original, 50, warm),
            LutRestoreOutcome::Restored
        );
        assert_eq!(backend.seam.tables.lock().unwrap()[&1], original);
    }

    #[test]
    fn a_restore_after_adoption_on_a_foreign_base_returns_the_as_found_table() {
        let foreign = warm_scale_table(64);
        let backend = backend(seam_with(foreign.dimmed(80)));
        let display = handle("cg-1");
        backend.adopt_baseline(&display, &foreign, 80, Tint::NEUTRAL);
        let session = backend.sessions.lock().unwrap();
        let entry = session.get("mac-cg-1").unwrap();
        assert_eq!(entry.written_checksum, Some(foreign.dimmed(80).checksum()));
        drop(session);
        assert_eq!(backend.restore(&display).unwrap(), RestoreOutcome::Restored);
        assert_eq!(backend.seam.tables.lock().unwrap()[&1], foreign);
    }

    #[test]
    fn a_restart_accepts_a_table_composed_on_a_rebound_base() {
        let foreign = warm_scale_table(64);
        let calibration = identity_table(64, 200);
        let warm = Tint::from_kelvin(3500);
        let shared = Arc::new(Mutex::new(HashMap::from([(1, foreign.clone())])));
        let display = handle("cg-1");
        let first = backend(FakeCgSeam {
            tables: Arc::clone(&shared),
        });
        first.adopt_baseline(&display, &foreign, 100, Tint::NEUTRAL);
        assert!(first.rebind_compose_base(&display, &calibration, false, true));
        first.set_tint(&display, warm).unwrap();
        assert_eq!(shared.lock().unwrap()[&1], calibration.tinted(warm));
        drop(first);
        let second = backend(FakeCgSeam {
            tables: Arc::clone(&shared),
        });
        second.adopt_baseline(&display, &foreign, 100, Tint::NEUTRAL);
        assert!(second.rebind_compose_base(&display, &calibration, false, true));
        assert_eq!(
            second.write_guarded(&display, &foreign, 100, warm),
            LutRestoreOutcome::Restored
        );
        assert_eq!(shared.lock().unwrap()[&1], foreign);
    }

    #[test]
    fn a_rebound_base_is_what_the_next_tint_composes_on() {
        let foreign = warm_scale_table(64);
        let backend = backend(seam_with(foreign));
        let display = handle("cg-1");
        let warm = Tint::from_kelvin(3500);
        backend.set_tint(&display, warm).unwrap();
        let calibration = identity_table(64, 200);
        assert!(backend.rebind_compose_base(&display, &calibration, false, true));
        backend.set_tint(&display, warm).unwrap();
        let written = backend.seam.tables.lock().unwrap()[&1].clone();
        assert_eq!(written, calibration.tinted(warm));
    }

    #[test]
    fn a_guarded_write_refuses_when_the_live_table_moved() {
        let original = identity_table(4, 100);
        let backend = backend(seam_with(original.clone()));
        let display = handle("cg-1");
        let error = backend
            .set_gamma_adjustment_guarded(&display, 50, Tint::NEUTRAL, original.checksum() + 1)
            .unwrap_err();
        assert!(error.is_gamma_changed_under_write());
        assert_eq!(backend.seam.tables.lock().unwrap()[&1], original);
    }

    #[test]
    fn a_guarded_write_with_a_matching_expectation_writes_and_verifies() {
        let original = identity_table(4, 100);
        let backend = backend(seam_with(original.clone()));
        let display = handle("cg-1");
        backend
            .set_gamma_adjustment_guarded(&display, 50, Tint::NEUTRAL, original.checksum())
            .unwrap();
        assert_eq!(backend.seam.tables.lock().unwrap()[&1], original.dimmed(50));
        let session = backend.sessions.lock().unwrap();
        let entry = session.get("mac-cg-1").unwrap();
        assert_eq!(entry.written_checksum, Some(original.dimmed(50).checksum()));
        assert_eq!(entry.written_value, Some(50));
        assert_eq!(entry.written_tint, Tint::NEUTRAL);
    }
}
