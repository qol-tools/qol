use std::ffi::c_void;

use sha2::{Digest, Sha256};

use super::DisplayEnumerator;
use crate::display::{
    cg_display_id_from_connector, validate_layout, DisplayError, DisplayHandle, DisplayMode,
    DisplayOps, DisplayPlacement, DisplaySnapshot,
};
use crate::geometry::MonitorBounds;

const MAX_DISPLAYS: u32 = 16;
const CONFIGURE_FOR_SESSION: u32 = 1;

#[repr(C)]
struct CgPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
struct CgSize {
    width: f64,
    height: f64,
}

#[repr(C)]
struct CgRect {
    origin: CgPoint,
    size: CgSize,
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGGetOnlineDisplayList(
        max_displays: u32,
        online_displays: *mut u32,
        display_count: *mut u32,
    ) -> i32;
    fn CGDisplayVendorNumber(display: u32) -> u32;
    fn CGDisplayModelNumber(display: u32) -> u32;
    fn CGDisplaySerialNumber(display: u32) -> u32;
    fn CGDisplayIsBuiltin(display: u32) -> bool;
    fn CGDisplayIsMain(display: u32) -> bool;
    fn CGDisplayBounds(display: u32) -> CgRect;
    fn CGDisplayCopyDisplayMode(display: u32) -> *mut c_void;
    fn CGDisplayModeGetIODisplayModeID(mode: *mut c_void) -> i32;
    fn CGDisplayModeGetWidth(mode: *mut c_void) -> usize;
    fn CGDisplayModeGetHeight(mode: *mut c_void) -> usize;
    fn CGDisplayModeGetRefreshRate(mode: *mut c_void) -> f64;
    fn CGDisplayCopyAllDisplayModes(display: u32, options: *const c_void) -> *const c_void;
    fn CGBeginDisplayConfiguration(config: *mut *mut c_void) -> i32;
    fn CGConfigureDisplayOrigin(config: *mut c_void, display: u32, x: i32, y: i32) -> i32;
    fn CGCompleteDisplayConfiguration(config: *mut c_void, option: u32) -> i32;
    fn CGCancelDisplayConfiguration(config: *mut c_void) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    fn CFRelease(value: *const c_void);
}

pub struct Platform;

impl DisplayEnumerator for Platform {
    fn enumerate(&self) -> Result<Vec<DisplayHandle>, DisplayError> {
        let display_ids = online_display_ids()?;
        let mut handles = Vec::with_capacity(display_ids.len());
        for display_id in display_ids {
            let vendor = unsafe { CGDisplayVendorNumber(display_id) };
            let model = unsafe { CGDisplayModelNumber(display_id) };
            let serial = unsafe { CGDisplaySerialNumber(display_id) };
            let builtin = unsafe { CGDisplayIsBuiltin(display_id) };
            handles.push(derive_handle(vendor, model, serial, display_id, builtin));
        }
        Ok(handles)
    }

    fn snapshot(&self) -> Result<Vec<DisplaySnapshot>, DisplayError> {
        let mut seam = CoreGraphicsSeam { config: None };
        snapshot_with(&mut seam)
    }
}

impl DisplayOps for Platform {
    fn modes(&self, handle: &DisplayHandle) -> Result<Vec<DisplayMode>, DisplayError> {
        let mut seam = CoreGraphicsSeam { config: None };
        modes_with(&mut seam, handle)
    }

    fn set_mode(&self, _handle: &DisplayHandle, _mode: &DisplayMode) -> Result<(), DisplayError> {
        Err(DisplayError::Unsupported {
            capability: "modes",
            reason: "mode writing is not exposed on macOS without private display APIs".into(),
        })
    }

    fn set_layout(&self, placements: &[DisplayPlacement]) -> Result<(), DisplayError> {
        let mut seam = CoreGraphicsSeam { config: None };
        set_layout_with(&mut seam, placements)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CgDisplayState {
    display_id: u32,
    vendor: u32,
    model: u32,
    serial: u32,
    builtin: bool,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    main: bool,
    current_mode: Option<CgModeState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CgModeState {
    token: u64,
    width: u32,
    height: u32,
    refresh_hz: u32,
}

trait CgDisplaySeam {
    fn displays(&self) -> Result<Vec<CgDisplayState>, DisplayError>;
    fn modes(&self, display_id: u32) -> Result<Vec<CgModeState>, DisplayError>;
    fn begin(&mut self) -> Result<(), DisplayError>;
    fn configure_origin(&mut self, display_id: u32, x: i32, y: i32) -> Result<(), DisplayError>;
    fn complete(&mut self) -> Result<(), DisplayError>;
    fn cancel(&mut self) -> Result<(), DisplayError>;
}

struct CoreGraphicsSeam {
    config: Option<*mut c_void>,
}

impl CgDisplaySeam for CoreGraphicsSeam {
    fn displays(&self) -> Result<Vec<CgDisplayState>, DisplayError> {
        let display_ids = online_display_ids()?;
        let mut states = Vec::with_capacity(display_ids.len());
        for display_id in display_ids {
            let bounds = unsafe { CGDisplayBounds(display_id) };
            states.push(CgDisplayState {
                display_id,
                vendor: unsafe { CGDisplayVendorNumber(display_id) },
                model: unsafe { CGDisplayModelNumber(display_id) },
                serial: unsafe { CGDisplaySerialNumber(display_id) },
                builtin: unsafe { CGDisplayIsBuiltin(display_id) },
                x: bounds.origin.x as i32,
                y: bounds.origin.y as i32,
                width: bounds.size.width as u32,
                height: bounds.size.height as u32,
                main: unsafe { CGDisplayIsMain(display_id) },
                current_mode: current_mode(display_id),
            });
        }
        Ok(states)
    }

    fn modes(&self, display_id: u32) -> Result<Vec<CgModeState>, DisplayError> {
        let array = unsafe { CGDisplayCopyAllDisplayModes(display_id, std::ptr::null()) };
        if array.is_null() {
            return Err(DisplayError::Io(std::io::Error::other(format!(
                "CGDisplayCopyAllDisplayModes failed for display {display_id}"
            ))));
        }
        let count = unsafe { CFArrayGetCount(array) };
        let mut modes = Vec::new();
        for index in 0..count {
            let value = unsafe { CFArrayGetValueAtIndex(array, index) };
            if value.is_null() {
                continue;
            }
            modes.push(read_mode(value.cast_mut()));
        }
        unsafe { CFRelease(array) };
        Ok(modes)
    }

    fn begin(&mut self) -> Result<(), DisplayError> {
        let mut config = std::ptr::null_mut();
        let error = unsafe { CGBeginDisplayConfiguration(&mut config) };
        if error != 0 {
            return Err(core_graphics_failed("CGBeginDisplayConfiguration", error));
        }
        self.config = Some(config);
        Ok(())
    }

    fn configure_origin(&mut self, display_id: u32, x: i32, y: i32) -> Result<(), DisplayError> {
        let Some(config) = self.config else {
            return Err(DisplayError::Io(std::io::Error::other(
                "no CoreGraphics display configuration is open",
            )));
        };
        let error = unsafe { CGConfigureDisplayOrigin(config, display_id, x, y) };
        if error != 0 {
            return Err(core_graphics_failed("CGConfigureDisplayOrigin", error));
        }
        Ok(())
    }

    fn complete(&mut self) -> Result<(), DisplayError> {
        let Some(config) = self.config else {
            return Err(DisplayError::Io(std::io::Error::other(
                "no CoreGraphics display configuration is open",
            )));
        };
        let error = unsafe { CGCompleteDisplayConfiguration(config, CONFIGURE_FOR_SESSION) };
        if error != 0 {
            return Err(core_graphics_failed(
                "CGCompleteDisplayConfiguration",
                error,
            ));
        }
        self.config = None;
        Ok(())
    }

    fn cancel(&mut self) -> Result<(), DisplayError> {
        let Some(config) = self.config.take() else {
            return Ok(());
        };
        let error = unsafe { CGCancelDisplayConfiguration(config) };
        if error != 0 {
            return Err(core_graphics_failed("CGCancelDisplayConfiguration", error));
        }
        Ok(())
    }
}

fn online_display_ids() -> Result<Vec<u32>, DisplayError> {
    let mut display_ids = [0u32; MAX_DISPLAYS as usize];
    let mut display_count = 0u32;
    let error = unsafe {
        CGGetOnlineDisplayList(MAX_DISPLAYS, display_ids.as_mut_ptr(), &mut display_count)
    };
    if error != 0 {
        return Err(DisplayError::Io(std::io::Error::other(format!(
            "CGGetOnlineDisplayList failed with {error}"
        ))));
    }
    Ok(display_ids[..display_count as usize].to_vec())
}

fn current_mode(display_id: u32) -> Option<CgModeState> {
    let mode = unsafe { CGDisplayCopyDisplayMode(display_id) };
    if mode.is_null() {
        return None;
    }
    let state = read_mode(mode);
    unsafe { CFRelease(mode) };
    Some(state)
}

fn read_mode(mode: *mut c_void) -> CgModeState {
    let token = unsafe { CGDisplayModeGetIODisplayModeID(mode) } as u64;
    let width = unsafe { CGDisplayModeGetWidth(mode) } as u32;
    let height = unsafe { CGDisplayModeGetHeight(mode) } as u32;
    let refresh = unsafe { CGDisplayModeGetRefreshRate(mode) };
    CgModeState {
        token,
        width,
        height,
        refresh_hz: refresh.round() as u32,
    }
}

fn core_graphics_failed(operation: &str, error: i32) -> DisplayError {
    DisplayError::Io(std::io::Error::other(format!(
        "{operation} failed with CGError {error}"
    )))
}

fn mode_from_cg(mode: &CgModeState) -> DisplayMode {
    DisplayMode {
        token: mode.token,
        width: mode.width,
        height: mode.height,
        refresh_hz: mode.refresh_hz,
    }
}

fn snapshot_with<S: CgDisplaySeam>(seam: &mut S) -> Result<Vec<DisplaySnapshot>, DisplayError> {
    let displays = seam.displays()?;
    let mut snapshots = Vec::with_capacity(displays.len());
    for display in displays {
        snapshots.push(DisplaySnapshot {
            handle: derive_handle(
                display.vendor,
                display.model,
                display.serial,
                display.display_id,
                display.builtin,
            ),
            bounds: MonitorBounds {
                x: display.x as f32,
                y: display.y as f32,
                width: display.width as f32,
                height: display.height as f32,
            },
            primary: display.main,
            mode: display.current_mode.as_ref().map(mode_from_cg),
        });
    }
    Ok(snapshots)
}

fn modes_with<S: CgDisplaySeam>(
    seam: &mut S,
    handle: &DisplayHandle,
) -> Result<Vec<DisplayMode>, DisplayError> {
    let connector = handle.connector();
    let Some(display_id) = cg_display_id_from_connector(connector) else {
        return Err(DisplayError::NotFound {
            capability: "modes",
            selector: connector.to_string(),
        });
    };
    let modes = seam.modes(display_id)?;
    if modes.is_empty() {
        return Err(DisplayError::Unsupported {
            capability: "modes",
            reason: format!("display {display_id} reports no selectable modes"),
        });
    }
    Ok(modes.iter().map(mode_from_cg).collect())
}

fn set_layout_with<S: CgDisplaySeam>(
    seam: &mut S,
    placements: &[DisplayPlacement],
) -> Result<(), DisplayError> {
    validate_layout(placements)?;
    let displays = seam.displays()?;
    let mut targets = Vec::with_capacity(placements.len());
    for placement in placements {
        let connector = placement.handle.connector();
        let Some(display_id) = cg_display_id_from_connector(connector) else {
            return Err(DisplayError::NotFound {
                capability: "layout",
                selector: connector.to_string(),
            });
        };
        if !displays.iter().any(|d| d.display_id == display_id) {
            return Err(DisplayError::NotFound {
                capability: "layout",
                selector: connector.to_string(),
            });
        }
        targets.push((placement, display_id));
    }
    let anchor = placements
        .iter()
        .find(|placement| placement.primary)
        .ok_or_else(|| DisplayError::LayoutInvalid {
            reason: "the layout has no primary display".into(),
        })?;
    let mut translated = Vec::with_capacity(targets.len());
    for (placement, display_id) in &targets {
        let x = i64::from(placement.x) - i64::from(anchor.x);
        let y = i64::from(placement.y) - i64::from(anchor.y);
        let x = i32::try_from(x).map_err(|_| DisplayError::LayoutInvalid {
            reason: format!(
                "the translated position for {} leaves the supported range",
                placement.handle.connector()
            ),
        })?;
        let y = i32::try_from(y).map_err(|_| DisplayError::LayoutInvalid {
            reason: format!(
                "the translated position for {} leaves the supported range",
                placement.handle.connector()
            ),
        })?;
        translated.push((*display_id, x, y));
    }
    seam.begin()?;
    for (display_id, x, y) in translated {
        if let Err(error) = seam.configure_origin(display_id, x, y) {
            let _ = seam.cancel();
            return Err(error);
        }
    }
    if let Err(error) = seam.complete() {
        let _ = seam.cancel();
        return Err(error);
    }
    Ok(())
}

fn derive_handle(
    vendor: u32,
    model: u32,
    serial: u32,
    display_id: u32,
    builtin: bool,
) -> DisplayHandle {
    let digest: [u8; 32] = Sha256::digest(format!("{vendor}:{model}:{serial}").as_bytes()).into();
    let mut hash_bytes = [0u8; 8];
    hash_bytes.copy_from_slice(&digest[..8]);
    let hash = u64::from_be_bytes(hash_bytes);
    let connector = if builtin {
        format!("cg-{display_id}-builtin")
    } else {
        format!("cg-{display_id}")
    };
    DisplayHandle::new(format!("mac-{hash:016x}"), connector, None, serial == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum SeamCall {
        Begin,
        Configure(u32, i32, i32),
        Complete(u64),
        Cancel(u64),
    }

    struct FakeSeam {
        displays: Vec<CgDisplayState>,
        modes: Vec<(u32, Vec<CgModeState>)>,
        calls: Vec<SeamCall>,
        complete_fails: bool,
        configure_fails: Option<u32>,
        config: Option<u64>,
        next_config: u64,
    }

    fn fake_displays() -> Vec<CgDisplayState> {
        vec![
            CgDisplayState {
                display_id: 1,
                vendor: 123,
                model: 456,
                serial: 789,
                builtin: false,
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                main: true,
                current_mode: Some(CgModeState {
                    token: 7,
                    width: 1920,
                    height: 1080,
                    refresh_hz: 60,
                }),
            },
            CgDisplayState {
                display_id: 2,
                vendor: 123,
                model: 456,
                serial: 790,
                builtin: false,
                x: 1920,
                y: 0,
                width: 2560,
                height: 1440,
                main: false,
                current_mode: None,
            },
        ]
    }

    fn fake_modes() -> Vec<(u32, Vec<CgModeState>)> {
        vec![
            (
                1,
                vec![
                    CgModeState {
                        token: 5,
                        width: 1920,
                        height: 1080,
                        refresh_hz: 30,
                    },
                    CgModeState {
                        token: 7,
                        width: 1920,
                        height: 1080,
                        refresh_hz: 60,
                    },
                ],
            ),
            (
                2,
                vec![CgModeState {
                    token: 9,
                    width: 2560,
                    height: 1440,
                    refresh_hz: 75,
                }],
            ),
        ]
    }

    fn fake_seam() -> FakeSeam {
        FakeSeam {
            displays: fake_displays(),
            modes: fake_modes(),
            calls: Vec::new(),
            complete_fails: false,
            configure_fails: None,
            config: None,
            next_config: 0,
        }
    }

    impl CgDisplaySeam for FakeSeam {
        fn displays(&self) -> Result<Vec<CgDisplayState>, DisplayError> {
            Ok(self.displays.clone())
        }

        fn modes(&self, display_id: u32) -> Result<Vec<CgModeState>, DisplayError> {
            self.modes
                .iter()
                .find(|(id, _)| *id == display_id)
                .map(|(_, modes)| modes.clone())
                .ok_or_else(|| DisplayError::Unsupported {
                    capability: "modes",
                    reason: format!("no modes for display {display_id}"),
                })
        }

        fn begin(&mut self) -> Result<(), DisplayError> {
            self.next_config += 1;
            self.config = Some(self.next_config);
            self.calls.push(SeamCall::Begin);
            Ok(())
        }

        fn configure_origin(
            &mut self,
            display_id: u32,
            x: i32,
            y: i32,
        ) -> Result<(), DisplayError> {
            self.calls.push(SeamCall::Configure(display_id, x, y));
            if self.configure_fails == Some(display_id) {
                return Err(DisplayError::Io(std::io::Error::other(
                    "the fake configure failed",
                )));
            }
            Ok(())
        }

        fn complete(&mut self) -> Result<(), DisplayError> {
            let Some(config) = self.config else {
                return Err(DisplayError::Io(std::io::Error::other(
                    "no fake configuration is open",
                )));
            };
            self.calls.push(SeamCall::Complete(config));
            if self.complete_fails {
                return Err(DisplayError::Io(std::io::Error::other(
                    "the fake complete failed",
                )));
            }
            self.config = None;
            Ok(())
        }

        fn cancel(&mut self) -> Result<(), DisplayError> {
            let Some(config) = self.config.take() else {
                return Ok(());
            };
            self.calls.push(SeamCall::Cancel(config));
            Ok(())
        }
    }

    fn fake_placement(display_id: u32, x: i32, y: i32, primary: bool) -> DisplayPlacement {
        DisplayPlacement {
            handle: DisplayHandle::new(
                format!("mac-{display_id:016x}"),
                format!("cg-{display_id}"),
                None,
                false,
            ),
            x,
            y,
            primary,
        }
    }

    #[test]
    fn snapshot_maps_bounds_primary_and_current_mode() {
        let mut seam = fake_seam();
        let snapshots = snapshot_with(&mut seam).unwrap();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].handle, derive_handle(123, 456, 789, 1, false));
        assert_eq!(
            snapshots[0].bounds,
            MonitorBounds {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
            }
        );
        assert!(snapshots[0].primary);
        assert_eq!(
            snapshots[0].mode,
            Some(DisplayMode {
                token: 7,
                width: 1920,
                height: 1080,
                refresh_hz: 60,
            })
        );
        assert_eq!(snapshots[1].handle, derive_handle(123, 456, 790, 2, false));
        assert_eq!(
            snapshots[1].bounds,
            MonitorBounds {
                x: 1920.0,
                y: 0.0,
                width: 2560.0,
                height: 1440.0,
            }
        );
        assert!(!snapshots[1].primary);
        assert_eq!(snapshots[1].mode, None);
    }

    #[test]
    fn modes_list_every_mode_including_the_current_one() {
        let mut seam = fake_seam();
        let handle = DisplayHandle::new("mac-x".into(), "cg-1".into(), None, false);
        let modes = modes_with(&mut seam, &handle).unwrap();
        assert_eq!(
            modes.iter().map(|mode| mode.token).collect::<Vec<_>>(),
            vec![5, 7]
        );
        assert_eq!(modes[1].refresh_hz, 60);
    }

    #[test]
    fn modes_refuse_an_empty_mode_list() {
        let mut seam = fake_seam();
        seam.modes = vec![(1, Vec::new())];
        let handle = DisplayHandle::new("mac-x".into(), "cg-1".into(), None, false);
        assert!(matches!(
            modes_with(&mut seam, &handle),
            Err(DisplayError::Unsupported { .. })
        ));
    }

    #[test]
    fn modes_refuse_a_connector_without_a_display_id() {
        let mut seam = fake_seam();
        let handle = DisplayHandle::new("mac-x".into(), "card0-DP-1".into(), None, false);
        match modes_with(&mut seam, &handle) {
            Err(DisplayError::NotFound {
                capability,
                selector,
            }) => {
                assert_eq!(capability, "modes");
                assert_eq!(selector, "card0-DP-1");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn set_layout_translates_placements_so_the_primary_lands_at_the_origin() {
        let mut seam = fake_seam();
        let placements = vec![
            fake_placement(2, 0, 0, false),
            fake_placement(1, 1920, 120, true),
        ];
        set_layout_with(&mut seam, &placements).unwrap();
        assert_eq!(
            seam.calls,
            vec![
                SeamCall::Begin,
                SeamCall::Configure(2, -1920, -120),
                SeamCall::Configure(1, 0, 0),
                SeamCall::Complete(1),
            ]
        );
    }

    #[test]
    fn set_layout_keeps_placements_when_the_primary_is_at_the_origin() {
        let mut seam = fake_seam();
        let placements = vec![
            fake_placement(1, 0, 0, true),
            fake_placement(2, 1920, 0, false),
        ];
        set_layout_with(&mut seam, &placements).unwrap();
        assert_eq!(
            seam.calls,
            vec![
                SeamCall::Begin,
                SeamCall::Configure(1, 0, 0),
                SeamCall::Configure(2, 1920, 0),
                SeamCall::Complete(1),
            ]
        );
    }

    #[test]
    fn set_layout_cancels_the_same_config_when_complete_fails() {
        let mut seam = fake_seam();
        seam.complete_fails = true;
        let placements = vec![fake_placement(1, 0, 0, true)];
        assert!(set_layout_with(&mut seam, &placements).is_err());
        assert_eq!(
            seam.calls,
            vec![
                SeamCall::Begin,
                SeamCall::Configure(1, 0, 0),
                SeamCall::Complete(1),
                SeamCall::Cancel(1),
            ]
        );
    }

    #[test]
    fn set_layout_cancels_when_a_configure_fails() {
        let mut seam = fake_seam();
        seam.configure_fails = Some(1);
        let placements = vec![fake_placement(1, 0, 0, true)];
        assert!(set_layout_with(&mut seam, &placements).is_err());
        assert_eq!(
            seam.calls,
            vec![
                SeamCall::Begin,
                SeamCall::Configure(1, 0, 0),
                SeamCall::Cancel(1),
            ]
        );
    }

    #[test]
    fn set_layout_refuses_translated_origins_that_leave_i32() {
        let cases = [
            (i32::MIN, 0, 0, 0),
            (0, i32::MIN, 0, 0),
            (i32::MAX, 0, i32::MIN, 0),
            (0, i32::MAX, 0, i32::MIN),
            (i32::MIN, i32::MIN, i32::MAX, i32::MAX),
        ];
        for (anchor_x, anchor_y, x, y) in cases {
            let mut seam = fake_seam();
            let placements = vec![
                fake_placement(1, anchor_x, anchor_y, true),
                fake_placement(2, x, y, false),
            ];
            assert!(
                matches!(
                    set_layout_with(&mut seam, &placements),
                    Err(DisplayError::LayoutInvalid { .. })
                ),
                "({anchor_x}, {anchor_y}) with ({x}, {y}) should be refused"
            );
            assert!(seam.calls.is_empty());
        }
        let mut seam = fake_seam();
        let placements = vec![
            fake_placement(1, i32::MIN, i32::MIN, true),
            fake_placement(2, i32::MIN, i32::MIN, false),
        ];
        set_layout_with(&mut seam, &placements).unwrap();
        assert_eq!(
            seam.calls,
            vec![
                SeamCall::Begin,
                SeamCall::Configure(1, 0, 0),
                SeamCall::Configure(2, 0, 0),
                SeamCall::Complete(1),
            ]
        );
    }

    #[test]
    fn set_layout_refuses_an_unknown_display() {
        let mut seam = fake_seam();
        let placements = vec![fake_placement(9, 0, 0, true)];
        match set_layout_with(&mut seam, &placements) {
            Err(DisplayError::NotFound {
                capability,
                selector,
            }) => {
                assert_eq!(capability, "layout");
                assert_eq!(selector, "cg-9");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
        assert!(seam.calls.is_empty());
    }

    #[test]
    fn set_mode_is_unsupported() {
        let handle = DisplayHandle::new("mac-x".into(), "cg-1".into(), None, false);
        let mode = DisplayMode {
            token: 7,
            width: 1920,
            height: 1080,
            refresh_hz: 60,
        };
        match Platform.set_mode(&handle, &mode) {
            Err(DisplayError::Unsupported { capability, .. }) => assert_eq!(capability, "modes"),
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn derive_handle_pins_stable_id() {
        let handle = derive_handle(123, 456, 789, 1, false);
        assert_eq!(handle.id(), "mac-a879cdb88eb95acd");
    }

    #[test]
    fn derive_handle_serial_zero_marks_identity_unstable() {
        let handle = derive_handle(123, 456, 0, 1, false);
        assert!(handle.identity_unstable());
    }

    #[test]
    fn derive_handle_builtin_changes_connector_suffix() {
        let handle = derive_handle(123, 456, 789, 1, true);
        assert_eq!(handle.connector(), "cg-1-builtin");
    }

    #[test]
    fn derive_handle_same_tuple_is_equal() {
        let first = derive_handle(123, 456, 789, 1, false);
        let second = derive_handle(123, 456, 789, 1, false);
        assert_eq!(first, second);
    }

    #[test]
    fn derive_handle_serial_change_changes_id() {
        let first = derive_handle(123, 456, 789, 1, false);
        let second = derive_handle(123, 456, 790, 1, false);
        assert_ne!(first.id(), second.id());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn enumerate_returns_ok_on_this_host() {
        assert!(Platform.enumerate().is_ok());
    }
}
