use anyhow::{anyhow, Result};
use std::ffi::{c_void, CStr};

use crate::Rect;

mod ffi;

const LIBRARY: &CStr = c"libnvidia-fbc.so.1";
const MILLIS_PER_SECOND: u32 = 1000;
const GEFORCE_UNLOCK_KEY: [u8; 16] = [
    0xac, 0x10, 0xc9, 0x2e, 0xa5, 0xe6, 0x87, 0x4f, 0x8f, 0x4b, 0xf4, 0x61, 0xf8, 0x56, 0x27, 0xe9,
];

struct Api {
    get_last_error_str: ffi::GetLastErrorStr,
    create_handle: ffi::CreateHandle,
    destroy_handle: ffi::DestroyHandle,
    get_status: ffi::GetStatus,
    create_capture_session: ffi::CreateCaptureSession,
    destroy_capture_session: ffi::DestroyCaptureSession,
    to_sys_set_up: ffi::ToSysSetUp,
    to_sys_grab_frame: ffi::ToSysGrabFrame,
}

impl Api {
    fn load() -> Result<Self> {
        // SAFETY: dlopen receives a NUL-terminated library name; the handle is never closed, so the symbols stay valid for the process.
        let library = unsafe { libc::dlopen(LIBRARY.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if library.is_null() {
            return Err(anyhow!("{} is not installed", LIBRARY.to_string_lossy()));
        }
        // SAFETY: each symbol is looked up by its exported NvFBC 1.7 name and cast to the signature the NvFBC header declares for it.
        unsafe {
            Ok(Self {
                get_last_error_str: std::mem::transmute::<*mut c_void, ffi::GetLastErrorStr>(
                    symbol(library, c"NvFBCGetLastErrorStr")?,
                ),
                create_handle: std::mem::transmute::<*mut c_void, ffi::CreateHandle>(symbol(
                    library,
                    c"NvFBCCreateHandle",
                )?),
                destroy_handle: std::mem::transmute::<*mut c_void, ffi::DestroyHandle>(symbol(
                    library,
                    c"NvFBCDestroyHandle",
                )?),
                get_status: std::mem::transmute::<*mut c_void, ffi::GetStatus>(symbol(
                    library,
                    c"NvFBCGetStatus",
                )?),
                create_capture_session: std::mem::transmute::<*mut c_void, ffi::CreateCaptureSession>(
                    symbol(library, c"NvFBCCreateCaptureSession")?,
                ),
                destroy_capture_session: std::mem::transmute::<
                    *mut c_void,
                    ffi::DestroyCaptureSession,
                >(symbol(
                    library,
                    c"NvFBCDestroyCaptureSession",
                )?),
                to_sys_set_up: std::mem::transmute::<*mut c_void, ffi::ToSysSetUp>(symbol(
                    library,
                    c"NvFBCToSysSetUp",
                )?),
                to_sys_grab_frame: std::mem::transmute::<*mut c_void, ffi::ToSysGrabFrame>(symbol(
                    library,
                    c"NvFBCToSysGrabFrame",
                )?),
            })
        }
    }

    fn last_error(&self, handle: ffi::SessionHandle) -> String {
        // SAFETY: NvFBC returns a NUL-terminated string it owns, or null.
        let message = unsafe { (self.get_last_error_str)(handle) };
        if message.is_null() {
            return "unknown NvFBC error".to_string();
        }
        // SAFETY: the pointer is non-null and NUL-terminated per the NvFBC contract.
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    }
}

fn symbol(library: *mut c_void, name: &CStr) -> Result<*mut c_void> {
    // SAFETY: library is a live dlopen handle and name is NUL-terminated.
    let address = unsafe { libc::dlsym(library, name.as_ptr()) };
    if address.is_null() {
        return Err(anyhow!(
            "{} lacks {}",
            LIBRARY.to_string_lossy(),
            name.to_string_lossy()
        ));
    }
    Ok(address)
}

fn sampling_interval_ms(framerate: u32) -> u32 {
    (MILLIS_PER_SECOND / framerate.max(1)).max(1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FrameFormat {
    Nv12,
    Bgra,
}

impl FrameFormat {
    pub(super) fn preferred_for(rect: &Rect) -> Self {
        if rect.w % 4 == 0 && rect.h % 2 == 0 {
            Self::Nv12
        } else {
            Self::Bgra
        }
    }

    pub(super) fn ffmpeg_pix_fmt(self) -> &'static str {
        match self {
            Self::Nv12 => "nv12",
            Self::Bgra => "bgra",
        }
    }

    fn buffer_format(self) -> u32 {
        match self {
            Self::Nv12 => ffi::BUFFER_FORMAT_NV12,
            Self::Bgra => ffi::BUFFER_FORMAT_BGRA,
        }
    }

    fn frame_len(self, rect: &Rect) -> usize {
        let pixels = rect.w.max(0) as usize * rect.h.max(0) as usize;
        match self {
            Self::Nv12 => pixels * 3 / 2,
            Self::Bgra => pixels * 4,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Tracking {
    kind: u32,
    output_id: u32,
    capture_box: ffi::NvBox,
}

fn tracking_for(rect: &Rect, outputs: &[(u32, ffi::NvBox)]) -> Result<Tracking> {
    let (Ok(x), Ok(y), Ok(w), Ok(h)) = (
        u32::try_from(rect.x),
        u32::try_from(rect.y),
        u32::try_from(rect.w),
        u32::try_from(rect.h),
    ) else {
        return Err(anyhow!("selection {rect:?} lies outside the X screen"));
    };
    let inside = outputs.iter().find(|(_, output)| {
        x >= output.x
            && y >= output.y
            && x + w <= output.x + output.w
            && y + h <= output.y + output.h
    });
    Ok(match inside {
        Some(&(output_id, output)) => Tracking {
            kind: ffi::TRACKING_OUTPUT,
            output_id,
            capture_box: if (x, y, w, h) == (output.x, output.y, output.w, output.h) {
                ffi::NvBox::default()
            } else {
                ffi::NvBox {
                    x: x - output.x,
                    y: y - output.y,
                    w,
                    h,
                }
            },
        },
        None => Tracking {
            kind: ffi::TRACKING_SCREEN,
            output_id: 0,
            capture_box: ffi::NvBox { x, y, w, h },
        },
    })
}

pub(super) struct Capture {
    api: Api,
    handle: ffi::SessionHandle,
    session_created: bool,
    buffer: Box<*mut c_void>,
    frame_len: usize,
    output_name: String,
}

impl Capture {
    pub(super) fn open(rect: &Rect, framerate: u32, format: FrameFormat) -> Result<Self> {
        let api = Api::load()?;
        let mut capture = Self {
            api,
            handle: 0,
            session_created: false,
            buffer: Box::new(std::ptr::null_mut()),
            frame_len: format.frame_len(rect),
            output_name: String::new(),
        };
        capture.create_handle()?;
        let mut status = ffi::GetStatusParams::new();
        // SAFETY: the handle is live and status is a correctly versioned NvFBC 1.7 struct.
        capture.check(unsafe { (capture.api.get_status)(capture.handle, &mut status) })?;
        if status.is_capture_possible != ffi::TRUE || status.can_create_now != ffi::TRUE {
            return Err(anyhow!("NvFBC cannot start a capture on this system now"));
        }
        let outputs: Vec<(u32, ffi::NvBox)> = status
            .outputs()
            .iter()
            .map(|output| (output.id, output.tracked_box))
            .collect();
        let tracking = tracking_for(rect, &outputs)?;
        capture.output_name = status
            .outputs()
            .iter()
            .find(|output| tracking.kind == ffi::TRACKING_OUTPUT && output.id == tracking.output_id)
            .map_or_else(|| "screen".to_string(), ffi::OutputInfo::name);
        let mut session = ffi::CreateCaptureSessionParams::new(
            tracking.kind,
            tracking.output_id,
            tracking.capture_box,
            sampling_interval_ms(framerate),
        );
        // SAFETY: the handle is live and session is a correctly versioned NvFBC 1.7 struct.
        capture
            .check(unsafe { (capture.api.create_capture_session)(capture.handle, &mut session) })?;
        capture.session_created = true;
        let buffer: *mut *mut c_void = &mut *capture.buffer;
        let mut setup = ffi::ToSysSetupParams::new(buffer, format.buffer_format());
        // SAFETY: the buffer slot is boxed, so NvFBC can keep writing the frame pointer into it for the session's life.
        capture.check(unsafe { (capture.api.to_sys_set_up)(capture.handle, &mut setup) })?;
        capture.grab()?;
        Ok(capture)
    }

    pub(super) fn output_name(&self) -> &str {
        &self.output_name
    }

    pub(super) fn grab(&mut self) -> Result<&[u8]> {
        let mut info = ffi::FrameGrabInfo::default();
        let mut grab = ffi::ToSysGrabFrameParams::new(&mut info);
        // SAFETY: the session is set up for system-memory frames and info outlives the call.
        self.check(unsafe { (self.api.to_sys_grab_frame)(self.handle, &mut grab) })?;
        if info.byte_size as usize != self.frame_len || self.buffer.is_null() {
            return Err(anyhow!(
                "NvFBC returned a {}x{} frame of {} bytes, expected {} bytes",
                info.width,
                info.height,
                info.byte_size,
                self.frame_len
            ));
        }
        // SAFETY: NvFBC owns a frame buffer of byte_size bytes at *buffer until the next grab.
        Ok(unsafe { std::slice::from_raw_parts((*self.buffer).cast::<u8>(), self.frame_len) })
    }

    fn create_handle(&mut self) -> Result<()> {
        let mut params = ffi::CreateHandleParams::new(None);
        // SAFETY: params is a correctly versioned NvFBC 1.7 struct and handle is a valid out pointer.
        if unsafe { (self.api.create_handle)(&mut self.handle, &mut params) } == ffi::SUCCESS {
            return Ok(());
        }
        let mut params = ffi::CreateHandleParams::new(Some(&GEFORCE_UNLOCK_KEY));
        // SAFETY: same as above; the unlock key is static, so it outlives the call.
        let status = unsafe { (self.api.create_handle)(&mut self.handle, &mut params) };
        self.check(status)
    }

    fn check(&self, status: ffi::Status) -> Result<()> {
        if status == ffi::SUCCESS {
            return Ok(());
        }
        Err(anyhow!(
            "NvFBC error {status}: {}",
            self.api.last_error(self.handle)
        ))
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        if self.handle == 0 {
            return;
        }
        if self.session_created {
            let mut params = ffi::VersionOnlyParams::new();
            // SAFETY: the capture session was created on this live handle.
            unsafe { (self.api.destroy_capture_session)(self.handle, &mut params) };
        }
        let mut params = ffi::VersionOnlyParams::new();
        // SAFETY: the handle is live and is destroyed exactly once.
        unsafe { (self.api.destroy_handle)(self.handle, &mut params) };
    }
}

#[cfg(test)]
mod tests {
    use super::{ffi, sampling_interval_ms, tracking_for, FrameFormat, Tracking};
    use crate::Rect;

    fn nv_box(x: u32, y: u32, w: u32, h: u32) -> ffi::NvBox {
        ffi::NvBox { x, y, w, h }
    }

    #[test]
    fn nv12_frames_need_a_width_of_four_and_cost_a_third_of_bgra() {
        let cases = [
            (2560, 1440, FrameFormat::Nv12, 5_529_600),
            (1920, 1080, FrameFormat::Nv12, 3_110_400),
            (1126, 724, FrameFormat::Bgra, 3_260_896),
            (1124, 724, FrameFormat::Nv12, 1_220_664),
        ];
        for (w, h, format, len) in cases {
            let rect = Rect { x: 0, y: 0, w, h };
            assert_eq!(FrameFormat::preferred_for(&rect), format, "{w}x{h}");
            assert_eq!(format.frame_len(&rect), len, "{w}x{h}");
        }
        let full = Rect {
            x: 0,
            y: 0,
            w: 2560,
            h: 1440,
        };
        assert_eq!(
            FrameFormat::Bgra.frame_len(&full),
            FrameFormat::Nv12.frame_len(&full) * 8 / 3
        );
    }

    #[test]
    fn tracking_follows_the_output_that_holds_the_selection() {
        let outputs = [
            (445, nv_box(0, 213, 1920, 1080)),
            (477, nv_box(1920, 0, 2560, 1440)),
        ];
        let cases = [
            (
                Rect {
                    x: 1920,
                    y: 0,
                    w: 2560,
                    h: 1440,
                },
                Tracking {
                    kind: ffi::TRACKING_OUTPUT,
                    output_id: 477,
                    capture_box: ffi::NvBox::default(),
                },
            ),
            (
                Rect {
                    x: 2000,
                    y: 100,
                    w: 800,
                    h: 600,
                },
                Tracking {
                    kind: ffi::TRACKING_OUTPUT,
                    output_id: 477,
                    capture_box: nv_box(80, 100, 800, 600),
                },
            ),
            (
                Rect {
                    x: 1800,
                    y: 300,
                    w: 400,
                    h: 200,
                },
                Tracking {
                    kind: ffi::TRACKING_SCREEN,
                    output_id: 0,
                    capture_box: nv_box(1800, 300, 400, 200),
                },
            ),
        ];
        for (rect, expected) in cases {
            assert_eq!(tracking_for(&rect, &outputs).unwrap(), expected, "{rect:?}");
        }
    }

    #[test]
    fn sampling_refreshes_at_least_once_per_grab() {
        let cases = [
            (1, 1000),
            (30, 33),
            (60, 16),
            (120, 8),
            (144, 6),
            (240, 4),
            (0, 1000),
        ];
        for (framerate, expected) in cases {
            let interval = sampling_interval_ms(framerate);
            assert_eq!(interval, expected, "{framerate} fps");
            if framerate > 0 {
                assert!(interval * framerate <= 1000, "{framerate} fps");
            }
        }
    }

    #[test]
    fn tracking_rejects_a_selection_off_the_screen() {
        let rect = Rect {
            x: -10,
            y: 0,
            w: 100,
            h: 100,
        };
        assert!(tracking_for(&rect, &[]).is_err());
    }
}
