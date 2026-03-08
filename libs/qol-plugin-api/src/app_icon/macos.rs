use super::RgbaImage;
use std::ffi::c_void;

type CGImageRef = *const c_void;

#[repr(C)]
#[derive(Copy, Clone)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct CGSize {
    width: f64,
    height: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGColorSpaceCreateDeviceRGB() -> *const c_void;
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: *const c_void,
        bitmap_info: u32,
    ) -> *const c_void;
    fn CGContextDrawImage(ctx: *const c_void, rect: CGRect, image: CGImageRef);
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *const c_void);
}

pub fn icon_for_bundle_id(bundle_id: &str, size: usize) -> Option<RgbaImage> {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSString;

    objc2::rc::autoreleasepool(|_pool| {
        let ws = NSWorkspace::sharedWorkspace();
        let ns_id = NSString::from_str(bundle_id);
        let url = ws.URLForApplicationWithBundleIdentifier(&ns_id)?;
        let path = url.path()?;
        let ns_image = ws.iconForFile(&path);
        nsimage_to_rgba(&ns_image, size)
    })
}

pub fn icon_for_pid(pid: i32, size: usize) -> Option<RgbaImage> {
    use objc2_app_kit::NSRunningApplication;

    objc2::rc::autoreleasepool(|_pool| {
        let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
        let ns_image = app.icon()?;
        nsimage_to_rgba(&ns_image, size)
    })
}

fn nsimage_to_rgba(ns_image: &objc2_app_kit::NSImage, size: usize) -> Option<RgbaImage> {
    let cg_image =
        unsafe { ns_image.CGImageForProposedRect_context_hints(std::ptr::null_mut(), None, None) }?;

    let img_ptr = &*cg_image as *const objc2_core_graphics::CGImage as CGImageRef;

    let row_bytes = size * 4;
    let mut buf = vec![0u8; size * row_bytes];

    let color_space = unsafe { CGColorSpaceCreateDeviceRGB() };
    if color_space.is_null() {
        return None;
    }

    // kCGImageAlphaPremultipliedFirst (2) | kCGBitmapByteOrder32Little (2 << 12 = 8192)
    // = BGRA premultiplied, little-endian 32-bit
    let bitmap_info: u32 = 2 | (2 << 12);

    let ctx = unsafe {
        CGBitmapContextCreate(
            buf.as_mut_ptr() as *mut c_void,
            size,
            size,
            8,
            row_bytes,
            color_space,
            bitmap_info,
        )
    };
    unsafe { CFRelease(color_space) };

    if ctx.is_null() {
        return None;
    }

    let draw_rect = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize {
            width: size as f64,
            height: size as f64,
        },
    };
    unsafe { CGContextDrawImage(ctx, draw_rect, img_ptr) };
    unsafe { CFRelease(ctx) };

    Some(RgbaImage {
        data: buf,
        width: size,
        height: size,
    })
}
