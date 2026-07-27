use anyhow::{anyhow, Context, Result};
use block2::RcBlock;
use objc2::encode::{Encode, Encoding};
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, sel};
use std::ffi::c_void;
use std::sync::{mpsc, OnceLock};
use std::time::{Duration, Instant};

use crate::Monitor;

const CAPTURE_TIMEOUT: Duration = Duration::from_secs(2);

type CGImageRef = *const c_void;
type CGDataProviderRef = *const c_void;
type CFDataRef = *const c_void;

#[derive(Debug)]
pub(super) struct NativeDisplayFrame {
    pub(super) pixels: Vec<u8>,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) capture_ms: u128,
    pub(super) copy_ms: u128,
    pub(super) total_ms: u128,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

unsafe impl Encode for CGPoint {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

unsafe impl Encode for CGSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

unsafe impl Encode for CGRect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[CGPoint::ENCODING, CGSize::ENCODING]);
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGImageGetWidth(image: CGImageRef) -> usize;
    fn CGImageGetHeight(image: CGImageRef) -> usize;
    fn CGImageGetBytesPerRow(image: CGImageRef) -> usize;
    fn CGImageGetDataProvider(image: CGImageRef) -> CGDataProviderRef;
    fn CGDataProviderCopyData(provider: CGDataProviderRef) -> CFDataRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFDataGetBytePtr(data: CFDataRef) -> *const u8;
    fn CFDataGetLength(data: CFDataRef) -> isize;
    fn CFRelease(value: *const c_void);
}

pub(super) fn capture_display(bounds: Monitor) -> Result<Option<NativeDisplayFrame>> {
    let Some(manager) = screenshot_manager() else {
        return Ok(None);
    };
    let started = Instant::now();
    let (tx, rx) = mpsc::sync_channel(1);
    let block = RcBlock::new(move |image: CGImageRef, error: *mut AnyObject| {
        let capture_ms = started.elapsed().as_millis();
        let copy_started = Instant::now();
        let result = copy_capture(image, error).map(|(pixels, width, height)| NativeDisplayFrame {
            pixels,
            width,
            height,
            capture_ms,
            copy_ms: copy_started.elapsed().as_millis(),
            total_ms: started.elapsed().as_millis(),
        });
        let _ = tx.send(result);
    });
    let rect = capture_rect(bounds);
    unsafe {
        let _: () = msg_send![
            manager,
            captureImageInRect: rect,
            completionHandler: &*block,
        ];
    }
    rx.recv_timeout(CAPTURE_TIMEOUT)
        .context("ScreenCaptureKit display capture timed out")?
        .map(Some)
}

fn screenshot_manager() -> Option<&'static AnyClass> {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    if !*AVAILABLE.get_or_init(screenshot_manager_available) {
        return None;
    }
    AnyClass::get(c"SCScreenshotManager")
}

fn screenshot_manager_available() -> bool {
    extern "C" {
        fn dlopen(path: *const i8, mode: i32) -> *mut c_void;
    }
    let handle = unsafe {
        dlopen(
            c"/System/Library/Frameworks/ScreenCaptureKit.framework/ScreenCaptureKit".as_ptr(),
            1,
        )
    };
    let Some(manager) = AnyClass::get(c"SCScreenshotManager") else {
        return false;
    };
    if handle.is_null() {
        return false;
    }
    unsafe {
        msg_send![
            manager,
            respondsToSelector: sel!(captureImageInRect:completionHandler:)
        ]
    }
}

fn capture_rect(bounds: Monitor) -> CGRect {
    CGRect {
        origin: CGPoint {
            x: f64::from(bounds.x),
            y: f64::from(bounds.y),
        },
        size: CGSize {
            width: f64::from(bounds.w),
            height: f64::from(bounds.h),
        },
    }
}

fn copy_capture(image: CGImageRef, error: *mut AnyObject) -> Result<(Vec<u8>, u32, u32)> {
    if image.is_null() {
        return Err(anyhow!(
            "ScreenCaptureKit returned no image: {}",
            unsafe { error_description(error) }.unwrap_or_else(|| "unknown error".into())
        ));
    }
    let width = unsafe { CGImageGetWidth(image) };
    let height = unsafe { CGImageGetHeight(image) };
    let bytes_per_row = unsafe { CGImageGetBytesPerRow(image) };
    let provider = unsafe { CGImageGetDataProvider(image) };
    if provider.is_null() {
        return Err(anyhow!("ScreenCaptureKit image has no data provider"));
    }
    let data = unsafe { CGDataProviderCopyData(provider) };
    if data.is_null() {
        return Err(anyhow!("ScreenCaptureKit image data could not be copied"));
    }
    let result = copy_image_data(data, width, height, bytes_per_row);
    unsafe { CFRelease(data) };
    result
}

fn copy_image_data(
    data: CFDataRef,
    width: usize,
    height: usize,
    bytes_per_row: usize,
) -> Result<(Vec<u8>, u32, u32)> {
    let length = usize::try_from(unsafe { CFDataGetLength(data) })
        .context("ScreenCaptureKit image data length is invalid")?;
    let pointer = unsafe { CFDataGetBytePtr(data) };
    if pointer.is_null() {
        return Err(anyhow!("ScreenCaptureKit image data is empty"));
    }
    let source = unsafe { std::slice::from_raw_parts(pointer, length) };
    let pixels = pack_bgra_rows(source, width, height, bytes_per_row)
        .context("ScreenCaptureKit image rows are invalid")?;
    let width = u32::try_from(width).context("ScreenCaptureKit image is too wide")?;
    let height = u32::try_from(height).context("ScreenCaptureKit image is too tall")?;
    Ok((pixels, width, height))
}

fn pack_bgra_rows(
    source: &[u8],
    width: usize,
    height: usize,
    bytes_per_row: usize,
) -> Option<Vec<u8>> {
    let packed_row = width.checked_mul(4)?;
    if width == 0 || height == 0 || bytes_per_row < packed_row {
        return None;
    }
    let required = bytes_per_row
        .checked_mul(height.checked_sub(1)?)?
        .checked_add(packed_row)?;
    source.get(..required)?;
    let mut pixels = Vec::with_capacity(packed_row.checked_mul(height)?);
    for row in 0..height {
        let start = row.checked_mul(bytes_per_row)?;
        pixels.extend_from_slice(source.get(start..start.checked_add(packed_row)?)?);
    }
    Some(pixels)
}

unsafe fn error_description(error: *mut AnyObject) -> Option<String> {
    if error.is_null() {
        return None;
    }
    let description: *const AnyObject = msg_send![error, localizedDescription];
    if description.is_null() {
        return None;
    }
    let pointer: *const i8 = msg_send![description, UTF8String];
    if pointer.is_null() {
        return None;
    }
    Some(
        std::ffi::CStr::from_ptr(pointer)
            .to_string_lossy()
            .into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::pack_bgra_rows;

    #[test]
    fn bgra_rows_are_packed_without_padding() {
        let source = (0..16).collect::<Vec<_>>();
        assert_eq!(pack_bgra_rows(&source, 2, 2, 8), Some(source));
    }

    #[test]
    fn bgra_rows_drop_source_padding() {
        let source = vec![1, 2, 3, 4, 9, 9, 5, 6, 7, 8];
        assert_eq!(
            pack_bgra_rows(&source, 1, 2, 6),
            Some(vec![1, 2, 3, 4, 5, 6, 7, 8])
        );
    }

    #[test]
    fn bgra_rows_reject_invalid_layouts() {
        let cases = [
            (&[][..], 0, 1, 4),
            (&[][..], 1, 0, 4),
            (&[0; 4][..], 2, 1, 4),
            (&[0; 7][..], 1, 2, 4),
        ];
        for (source, width, height, bytes_per_row) in cases {
            assert_eq!(pack_bgra_rows(source, width, height, bytes_per_row), None);
        }
    }
}
