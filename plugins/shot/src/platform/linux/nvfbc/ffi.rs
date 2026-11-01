#![allow(
    dead_code,
    reason = "these structs mirror NvFBC.h, and the driver reads and writes their fields through raw pointers"
)]

use std::ffi::{c_char, c_void, CStr};

pub(super) type Status = u32;
pub(super) type Bool = u32;
pub(super) type SessionHandle = u64;

pub(super) const SUCCESS: Status = 0;
pub(super) const TRUE: Bool = 1;
pub(super) const CAPTURE_TO_SYS: u32 = 0;
pub(super) const TRACKING_OUTPUT: u32 = 1;
pub(super) const TRACKING_SCREEN: u32 = 2;
pub(super) const BUFFER_FORMAT_BGRA: u32 = 5;
pub(super) const TOSYS_GRAB_NOWAIT: u32 = 1;
pub(super) const OUTPUT_MAX: usize = 5;
pub(super) const OUTPUT_NAME_LEN: usize = 128;

const API_VERSION: u32 = 7 | (1 << 8);

const fn struct_version<T>(version: u32) -> u32 {
    std::mem::size_of::<T>() as u32 | (version << 16) | (API_VERSION << 24)
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct NvBox {
    pub(super) x: u32,
    pub(super) y: u32,
    pub(super) w: u32,
    pub(super) h: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct Size {
    pub(super) w: u32,
    pub(super) h: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(super) struct FrameGrabInfo {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) byte_size: u32,
    pub(super) current_frame: u32,
    pub(super) is_new_frame: Bool,
    pub(super) timestamp_us: u64,
    pub(super) missed_frames: u32,
    pub(super) required_post_processing: Bool,
    pub(super) direct_capture: Bool,
}

#[repr(C)]
pub(super) struct CreateHandleParams {
    pub(super) version: u32,
    pub(super) private_data: *const c_void,
    pub(super) private_data_size: u32,
    pub(super) externally_managed_context: Bool,
    pub(super) glx_ctx: *mut c_void,
    pub(super) glx_fb_config: *mut c_void,
}

impl CreateHandleParams {
    pub(super) fn new(private_data: Option<&'static [u8]>) -> Self {
        Self {
            version: struct_version::<Self>(2),
            private_data: private_data.map_or(std::ptr::null(), |data| data.as_ptr().cast()),
            private_data_size: private_data.map_or(0, |data| data.len() as u32),
            externally_managed_context: 0,
            glx_ctx: std::ptr::null_mut(),
            glx_fb_config: std::ptr::null_mut(),
        }
    }
}

#[repr(C)]
pub(super) struct VersionOnlyParams {
    pub(super) version: u32,
}

impl VersionOnlyParams {
    pub(super) fn new() -> Self {
        Self {
            version: struct_version::<Self>(1),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct OutputInfo {
    pub(super) id: u32,
    pub(super) name: [c_char; OUTPUT_NAME_LEN],
    pub(super) tracked_box: NvBox,
}

impl OutputInfo {
    pub(super) fn name(&self) -> String {
        let bytes = self.name.map(|byte| byte as u8);
        CStr::from_bytes_until_nul(&bytes)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

#[repr(C)]
pub(super) struct GetStatusParams {
    pub(super) version: u32,
    pub(super) is_capture_possible: Bool,
    pub(super) currently_capturing: Bool,
    pub(super) can_create_now: Bool,
    pub(super) screen_size: Size,
    pub(super) xrandr_available: Bool,
    pub(super) outputs: [OutputInfo; OUTPUT_MAX],
    pub(super) output_count: u32,
    pub(super) nvfbc_version: u32,
    pub(super) in_modeset: Bool,
}

impl GetStatusParams {
    pub(super) fn new() -> Self {
        Self {
            version: struct_version::<Self>(2),
            is_capture_possible: 0,
            currently_capturing: 0,
            can_create_now: 0,
            screen_size: Size::default(),
            xrandr_available: 0,
            outputs: [OutputInfo {
                id: 0,
                name: [0; OUTPUT_NAME_LEN],
                tracked_box: NvBox::default(),
            }; OUTPUT_MAX],
            output_count: 0,
            nvfbc_version: 0,
            in_modeset: 0,
        }
    }

    pub(super) fn outputs(&self) -> &[OutputInfo] {
        &self.outputs[..(self.output_count as usize).min(OUTPUT_MAX)]
    }
}

#[repr(C)]
pub(super) struct CreateCaptureSessionParams {
    pub(super) version: u32,
    pub(super) capture_type: u32,
    pub(super) tracking_type: u32,
    pub(super) output_id: u32,
    pub(super) capture_box: NvBox,
    pub(super) frame_size: Size,
    pub(super) with_cursor: Bool,
    pub(super) disable_auto_modeset_recovery: Bool,
    pub(super) round_frame_size: Bool,
    pub(super) sampling_rate_ms: u32,
    pub(super) push_model: Bool,
    pub(super) allow_direct_capture: Bool,
}

impl CreateCaptureSessionParams {
    pub(super) fn new(
        tracking_type: u32,
        output_id: u32,
        capture_box: NvBox,
        sampling_rate_ms: u32,
    ) -> Self {
        Self {
            version: struct_version::<Self>(6),
            capture_type: CAPTURE_TO_SYS,
            tracking_type,
            output_id,
            capture_box,
            frame_size: Size::default(),
            with_cursor: TRUE,
            disable_auto_modeset_recovery: 0,
            round_frame_size: 0,
            sampling_rate_ms,
            push_model: 0,
            allow_direct_capture: 0,
        }
    }
}

#[repr(C)]
pub(super) struct ToSysSetupParams {
    pub(super) version: u32,
    pub(super) buffer_format: u32,
    pub(super) buffer: *mut *mut c_void,
    pub(super) with_diff_map: Bool,
    pub(super) diff_map: *mut *mut c_void,
    pub(super) diff_map_scaling_factor: u32,
    pub(super) diff_map_size: Size,
}

impl ToSysSetupParams {
    pub(super) fn new(buffer: *mut *mut c_void) -> Self {
        Self {
            version: struct_version::<Self>(3),
            buffer_format: BUFFER_FORMAT_BGRA,
            buffer,
            with_diff_map: 0,
            diff_map: std::ptr::null_mut(),
            diff_map_scaling_factor: 0,
            diff_map_size: Size::default(),
        }
    }
}

#[repr(C)]
pub(super) struct ToSysGrabFrameParams {
    pub(super) version: u32,
    pub(super) flags: u32,
    pub(super) frame_grab_info: *mut FrameGrabInfo,
    pub(super) timeout_ms: u32,
}

impl ToSysGrabFrameParams {
    pub(super) fn new(frame_grab_info: *mut FrameGrabInfo) -> Self {
        Self {
            version: struct_version::<Self>(2),
            flags: TOSYS_GRAB_NOWAIT,
            frame_grab_info,
            timeout_ms: 0,
        }
    }
}

pub(super) type GetLastErrorStr = unsafe extern "C" fn(SessionHandle) -> *const c_char;
pub(super) type CreateHandle =
    unsafe extern "C" fn(*mut SessionHandle, *mut CreateHandleParams) -> Status;
pub(super) type DestroyHandle =
    unsafe extern "C" fn(SessionHandle, *mut VersionOnlyParams) -> Status;
pub(super) type GetStatus = unsafe extern "C" fn(SessionHandle, *mut GetStatusParams) -> Status;
pub(super) type CreateCaptureSession =
    unsafe extern "C" fn(SessionHandle, *mut CreateCaptureSessionParams) -> Status;
pub(super) type DestroyCaptureSession =
    unsafe extern "C" fn(SessionHandle, *mut VersionOnlyParams) -> Status;
pub(super) type ToSysSetUp = unsafe extern "C" fn(SessionHandle, *mut ToSysSetupParams) -> Status;
pub(super) type ToSysGrabFrame =
    unsafe extern "C" fn(SessionHandle, *mut ToSysGrabFrameParams) -> Status;

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn layouts_and_versions_match_the_nvfbc_1_7_header() {
        let cases = [
            (size_of::<FrameGrabInfo>(), 48, 0),
            (
                size_of::<CreateHandleParams>(),
                40,
                CreateHandleParams::new(None).version,
            ),
            (
                size_of::<VersionOnlyParams>(),
                4,
                VersionOnlyParams::new().version,
            ),
            (size_of::<OutputInfo>(), 148, 0),
            (
                size_of::<GetStatusParams>(),
                780,
                GetStatusParams::new().version,
            ),
            (
                size_of::<CreateCaptureSessionParams>(),
                64,
                CreateCaptureSessionParams::new(TRACKING_OUTPUT, 0, NvBox::default(), 16).version,
            ),
            (
                size_of::<ToSysSetupParams>(),
                48,
                ToSysSetupParams::new(std::ptr::null_mut()).version,
            ),
            (
                size_of::<ToSysGrabFrameParams>(),
                24,
                ToSysGrabFrameParams::new(std::ptr::null_mut()).version,
            ),
        ];
        let expected_versions = [
            0,
            0x0702_0028,
            0x0701_0004,
            0,
            0x0702_030c,
            0x0706_0040,
            0x0703_0030,
            0x0702_0018,
        ];
        for ((size, expected_size, version), expected_version) in
            cases.into_iter().zip(expected_versions)
        {
            assert_eq!(size, expected_size);
            assert_eq!(version, expected_version);
        }
    }
}
