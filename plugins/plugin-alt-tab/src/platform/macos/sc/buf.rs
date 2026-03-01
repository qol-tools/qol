use super::CFRelease;
use std::ffi::c_void;

/// Retained CVPixelBuffer raw pointer that is safe to send across threads.
/// CVPixelBuffer is reference-counted with atomic operations (CoreFoundation),
/// making cross-thread transfer safe. Rust just doesn't know because it's a raw pointer.
pub struct SendCVBuf(pub(crate) *mut c_void);
unsafe impl Send for SendCVBuf {}

impl SendCVBuf {
    pub fn into_cvpixelbuffer(self) -> core_video::pixel_buffer::CVPixelBuffer {
        use core_foundation::base::TCFType;
        let buf = unsafe {
            core_video::pixel_buffer::CVPixelBuffer::wrap_under_create_rule(
                self.0 as core_video::pixel_buffer::CVPixelBufferRef,
            )
        };
        std::mem::forget(self);
        buf
    }

    /// Read raw BGRA pixels from the CVPixelBuffer. Consumes self (CFRelease on drop).
    /// Handles both BGRA (direct memcpy) and YUV420 BiPlanar (NV12 → BGRA conversion).
    /// Returns (bgra_data, width, height).
    pub fn read_bgra(self) -> Option<(Vec<u8>, usize, usize)> {
        #[link(name = "CoreVideo", kind = "framework")]
        extern "C" {
            fn CVPixelBufferLockBaseAddress(buf: *const c_void, flags: u64) -> i32;
            fn CVPixelBufferUnlockBaseAddress(buf: *const c_void, flags: u64) -> i32;
            fn CVPixelBufferGetWidth(buf: *const c_void) -> usize;
            fn CVPixelBufferGetHeight(buf: *const c_void) -> usize;
            fn CVPixelBufferGetPixelFormatType(buf: *const c_void) -> u32;
        }
        const READ_ONLY: u64 = 0x0000_0001;

        let ptr = self.0 as *const c_void;
        let w = unsafe { CVPixelBufferGetWidth(ptr) };
        let h = unsafe { CVPixelBufferGetHeight(ptr) };
        if w == 0 || h == 0 {
            return None;
        }

        let ret = unsafe { CVPixelBufferLockBaseAddress(ptr, READ_ONLY) };
        if ret != 0 {
            return None;
        }

        let fmt = unsafe { CVPixelBufferGetPixelFormatType(ptr) };
        let result = match fmt {
            0x4247_5241 => read_bgra_from_bgra(ptr, w, h),
            0x3432_3066 | 0x3432_3076 => read_bgra_from_yuv420(ptr, w, h),
            _ => {
                eprintln!("[alt-tab/sc] read_bgra: unsupported format 0x{:08x}", fmt);
                None
            }
        };

        unsafe { CVPixelBufferUnlockBaseAddress(ptr, READ_ONLY) };
        result
    }

    /// Read BGRA pixel data and convert to a RenderImage. Consumes self.
    pub fn to_render_image(self) -> Option<std::sync::Arc<gpui::RenderImage>> {
        let (bgra, w, h) = self.read_bgra()?;
        crate::preview::bgra_to_render_image(bgra, w, h)
    }
}

impl Drop for SendCVBuf {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0 as *const c_void) };
        }
    }
}

/// Read BGRA pixels from a locked CVPixelBuffer in BGRA format.
fn read_bgra_from_bgra(ptr: *const c_void, w: usize, h: usize) -> Option<(Vec<u8>, usize, usize)> {
    #[link(name = "CoreVideo", kind = "framework")]
    extern "C" {
        fn CVPixelBufferGetBaseAddress(buf: *const c_void) -> *const u8;
        fn CVPixelBufferGetBytesPerRow(buf: *const c_void) -> usize;
    }
    let base = unsafe { CVPixelBufferGetBaseAddress(ptr) };
    let stride = unsafe { CVPixelBufferGetBytesPerRow(ptr) };
    if base.is_null() || stride < w * 4 { return None; }
    let mut bgra = vec![0u8; w * h * 4];
    for row in 0..h {
        let src = unsafe { base.add(row * stride) };
        let dst = &mut bgra[row * w * 4..][..w * 4];
        unsafe { std::ptr::copy_nonoverlapping(src, dst.as_mut_ptr(), w * 4) };
    }
    Some((bgra, w, h))
}

/// Read BGRA pixels from a locked CVPixelBuffer in YUV420 BiPlanar format (NV12/NV12v).
fn read_bgra_from_yuv420(ptr: *const c_void, w: usize, h: usize) -> Option<(Vec<u8>, usize, usize)> {
    #[link(name = "CoreVideo", kind = "framework")]
    extern "C" {
        fn CVPixelBufferGetBaseAddressOfPlane(buf: *const c_void, plane: usize) -> *const u8;
        fn CVPixelBufferGetBytesPerRowOfPlane(buf: *const c_void, plane: usize) -> usize;
    }
    let y_base = unsafe { CVPixelBufferGetBaseAddressOfPlane(ptr, 0) };
    let y_stride = unsafe { CVPixelBufferGetBytesPerRowOfPlane(ptr, 0) };
    let uv_base = unsafe { CVPixelBufferGetBaseAddressOfPlane(ptr, 1) };
    let uv_stride = unsafe { CVPixelBufferGetBytesPerRowOfPlane(ptr, 1) };
    if y_base.is_null() || uv_base.is_null() { return None; }
    let mut bgra = vec![0u8; w * h * 4];
    for row in 0..h {
        let uv_row = row / 2;
        for col in 0..w {
            let y = unsafe { *y_base.add(row * y_stride + col) } as i32;
            let uv_off = uv_row * uv_stride + (col & !1);
            let cb = unsafe { *uv_base.add(uv_off) } as i32 - 128;
            let cr = unsafe { *uv_base.add(uv_off + 1) } as i32 - 128;
            // BT.601 full-range fixed-point
            let r = ((256 * y + 359 * cr + 128) >> 8).clamp(0, 255) as u8;
            let g = ((256 * y - 88 * cb - 183 * cr + 128) >> 8).clamp(0, 255) as u8;
            let b = ((256 * y + 454 * cb + 128) >> 8).clamp(0, 255) as u8;
            let idx = (row * w + col) * 4;
            bgra[idx] = b;
            bgra[idx + 1] = g;
            bgra[idx + 2] = r;
            bgra[idx + 3] = 255;
        }
    }
    Some((bgra, w, h))
}
