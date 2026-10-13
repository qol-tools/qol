use std::ffi::c_void;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::FALSE;
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP,
    BITMAPINFO, BITMAPINFOHEADER, BITMAPV5HEADER, BI_BITFIELDS, BI_RGB, DIB_RGB_COLORS, HBITMAP,
    HDC,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, GetIconInfo, HCURSOR, ICONINFO,
};

use crate::cursor::platform::shake::{scale_bilinear, scaled_dimension, scaled_raster_hotspot};

pub(super) const MAX_CURSOR_DIMENSION: u32 = 256;
const RGB_MASK: u32 = 0x00FF_FFFF;
const ALPHA_MASK: u32 = 0xFF00_0000;
const OPAQUE_BLACK: u32 = 0xFF00_0000;
const OPAQUE_WHITE: u32 = 0xFFFF_FFFF;
const INVERT_STAND_IN: u32 = 0xFF80_8080;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Raster {
    pub width: u32,
    pub height: u32,
    pub xhot: u32,
    pub yhot: u32,
    pub pixels: Vec<u32>,
}

impl Raster {
    pub fn longest_side(&self) -> u32 {
        self.width.max(self.height)
    }

    pub fn scaled_to(&self, width: u32, height: u32) -> Option<Raster> {
        let count = pixel_count(width, height)?;
        let mut pixels = vec![0; count];
        scale_bilinear(
            &self.pixels,
            self.width,
            self.height,
            &mut pixels,
            width,
            height,
        );
        Some(Raster {
            width,
            height,
            xhot: scaled_raster_hotspot(self.xhot, self.width, width),
            yhot: scaled_raster_hotspot(self.yhot, self.height, height),
            pixels,
        })
    }
}

pub(super) fn target_size(base_width: u32, base_height: u32, scale: f32) -> Option<(u32, u32)> {
    Some((
        scaled_dimension(base_width, scale, MAX_CURSOR_DIMENSION)?,
        scaled_dimension(base_height, scale, MAX_CURSOR_DIMENSION)?,
    ))
}

pub(super) fn capture(cursor: HCURSOR) -> Option<Raster> {
    let mut info: ICONINFO = unsafe { std::mem::zeroed() };
    if unsafe { GetIconInfo(cursor, &mut info) } == 0 {
        return None;
    }
    let bitmaps = IconBitmaps {
        color: info.hbmColor,
        mask: info.hbmMask,
    };
    let screen = ScreenDc::open()?;
    let (width, height, pixels) = if bitmaps.color.is_null() {
        let (width, double_height) = bitmap_size(bitmaps.mask)?;
        let height = double_height / 2;
        let planes = read_pixels(screen.0, bitmaps.mask, width, double_height)?;
        let split = pixel_count(width, height)?;
        let (and_plane, xor_plane) = planes.split_at(split);
        (width, height, monochrome_pixels(and_plane, xor_plane))
    } else {
        let (width, height) = bitmap_size(bitmaps.color)?;
        let mut color = read_pixels(screen.0, bitmaps.color, width, height)?;
        if lacks_alpha(&color) {
            let mask = read_pixels(screen.0, bitmaps.mask, width, height)?;
            apply_mask_alpha(&mut color, &mask);
        }
        (width, height, color)
    };
    Some(Raster {
        width,
        height,
        xhot: info.xHotspot.min(width.saturating_sub(1)),
        yhot: info.yHotspot.min(height.saturating_sub(1)),
        pixels,
    })
}

pub(super) fn create_cursor(raster: &Raster) -> Option<HCURSOR> {
    let count = pixel_count(raster.width, raster.height)?;
    if raster.pixels.len() < count {
        return None;
    }
    let width = i32::try_from(raster.width).ok()?;
    let height = i32::try_from(raster.height).ok()?;
    let mut header: BITMAPV5HEADER = unsafe { std::mem::zeroed() };
    header.bV5Size = std::mem::size_of::<BITMAPV5HEADER>() as u32;
    header.bV5Width = width;
    header.bV5Height = -height;
    header.bV5Planes = 1;
    header.bV5BitCount = 32;
    header.bV5Compression = BI_BITFIELDS;
    header.bV5RedMask = 0x00FF_0000;
    header.bV5GreenMask = 0x0000_FF00;
    header.bV5BlueMask = 0x0000_00FF;
    header.bV5AlphaMask = 0xFF00_0000;
    let screen = ScreenDc::open()?;
    let mut bits: *mut c_void = null_mut();
    let color = unsafe {
        CreateDIBSection(
            screen.0,
            (&header as *const BITMAPV5HEADER).cast::<BITMAPINFO>(),
            DIB_RGB_COLORS,
            &mut bits,
            null_mut(),
            0,
        )
    };
    drop(screen);
    let mask_bits = vec![0u8; mask_stride(raster.width) * raster.height as usize];
    let mask = unsafe { CreateBitmap(width, height, 1, 1, mask_bits.as_ptr().cast()) };
    let bitmaps = IconBitmaps { color, mask };
    if bitmaps.color.is_null() || bitmaps.mask.is_null() || bits.is_null() {
        return None;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(raster.pixels.as_ptr(), bits.cast::<u32>(), count);
    }
    let info = ICONINFO {
        fIcon: FALSE,
        xHotspot: raster.xhot,
        yHotspot: raster.yhot,
        hbmMask: bitmaps.mask,
        hbmColor: bitmaps.color,
    };
    let cursor = unsafe { CreateIconIndirect(&info) };
    (!cursor.is_null()).then_some(cursor)
}

struct IconBitmaps {
    color: HBITMAP,
    mask: HBITMAP,
}

impl Drop for IconBitmaps {
    fn drop(&mut self) {
        for bitmap in [self.color, self.mask] {
            if !bitmap.is_null() {
                unsafe { DeleteObject(bitmap) };
            }
        }
    }
}

struct ScreenDc(HDC);

impl ScreenDc {
    fn open() -> Option<Self> {
        let dc = unsafe { GetDC(null_mut()) };
        (!dc.is_null()).then_some(Self(dc))
    }
}

impl Drop for ScreenDc {
    fn drop(&mut self) {
        unsafe { ReleaseDC(null_mut(), self.0) };
    }
}

fn bitmap_size(bitmap: HBITMAP) -> Option<(u32, u32)> {
    let mut info: BITMAP = unsafe { std::mem::zeroed() };
    let read = unsafe {
        GetObjectW(
            bitmap,
            std::mem::size_of::<BITMAP>() as i32,
            (&mut info as *mut BITMAP).cast(),
        )
    };
    if read == 0 {
        return None;
    }
    let width = u32::try_from(info.bmWidth).ok()?;
    let height = u32::try_from(info.bmHeight).ok()?;
    pixel_count(width, height)?;
    Some((width, height))
}

fn read_pixels(dc: HDC, bitmap: HBITMAP, width: u32, height: u32) -> Option<Vec<u32>> {
    let count = pixel_count(width, height)?;
    let mut info: BITMAPINFO = unsafe { std::mem::zeroed() };
    info.bmiHeader = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: i32::try_from(width).ok()?,
        biHeight: -i32::try_from(height).ok()?,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        biSizeImage: 0,
        biXPelsPerMeter: 0,
        biYPelsPerMeter: 0,
        biClrUsed: 0,
        biClrImportant: 0,
    };
    let mut pixels = vec![0u32; count];
    let lines = unsafe {
        GetDIBits(
            dc,
            bitmap,
            0,
            height,
            pixels.as_mut_ptr().cast(),
            &mut info,
            DIB_RGB_COLORS,
        )
    };
    (u32::try_from(lines).ok() == Some(height)).then_some(pixels)
}

fn pixel_count(width: u32, height: u32) -> Option<usize> {
    if width == 0 || height == 0 {
        return None;
    }
    usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)
}

fn mask_stride(width: u32) -> usize {
    width.div_ceil(16) as usize * 2
}

fn lacks_alpha(pixels: &[u32]) -> bool {
    pixels.iter().all(|pixel| pixel & ALPHA_MASK == 0)
}

fn apply_mask_alpha(color: &mut [u32], mask: &[u32]) {
    for (pixel, mask) in color.iter_mut().zip(mask) {
        *pixel = if mask & RGB_MASK == 0 {
            (*pixel & RGB_MASK) | ALPHA_MASK
        } else {
            0
        };
    }
}

fn monochrome_pixels(and_plane: &[u32], xor_plane: &[u32]) -> Vec<u32> {
    and_plane
        .iter()
        .zip(xor_plane)
        .map(|(and, xor)| monochrome_pixel(*and, *xor))
        .collect()
}

fn monochrome_pixel(and: u32, xor: u32) -> u32 {
    match (and & RGB_MASK != 0, xor & RGB_MASK != 0) {
        (false, false) => OPAQUE_BLACK,
        (false, true) => OPAQUE_WHITE,
        (true, false) => 0,
        (true, true) => INVERT_STAND_IN,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monochrome_planes_map_to_argb() {
        let cases = [
            (0x0000_0000, 0x0000_0000, OPAQUE_BLACK),
            (0x0000_0000, 0x00FF_FFFF, OPAQUE_WHITE),
            (0x00FF_FFFF, 0x0000_0000, 0),
            (0x00FF_FFFF, 0x00FF_FFFF, INVERT_STAND_IN),
        ];
        for (and, xor, expected) in cases {
            assert_eq!(
                monochrome_pixel(and, xor),
                expected,
                "and={and:#x} xor={xor:#x}"
            );
        }
    }

    #[test]
    fn mask_alpha_fills_only_alphaless_color() {
        let cases: [(&[u32], bool); 4] = [
            (&[0x0012_3456, 0x0000_0000], true),
            (&[0x8012_3456, 0x0000_0000], false),
            (&[0xFF00_0000], false),
            (&[], true),
        ];
        for (pixels, expected) in cases {
            assert_eq!(lacks_alpha(pixels), expected, "pixels={pixels:x?}");
        }

        let mut color = [0x0012_3456, 0x00AB_CDEF];
        apply_mask_alpha(&mut color, &[0x0000_0000, 0x00FF_FFFF]);
        assert_eq!(color, [0xFF12_3456, 0]);
    }

    #[test]
    fn mask_rows_are_word_aligned() {
        let cases = [(1, 2), (16, 2), (17, 4), (32, 4), (33, 6), (256, 32)];
        for (width, expected) in cases {
            assert_eq!(mask_stride(width), expected, "width={width}");
        }
    }

    #[test]
    fn target_size_scales_and_clamps() {
        let cases = [
            ((32, 32, 1.0), Some((32, 32))),
            ((32, 32, 4.0), Some((128, 128))),
            ((48, 48, 4.0), Some((192, 192))),
            ((64, 64, 8.0), Some((256, 256))),
            ((32, 32, 0.0), None),
            ((32, 32, f32::NAN), None),
        ];
        for ((width, height, scale), expected) in cases {
            assert_eq!(
                target_size(width, height, scale),
                expected,
                "base={width}x{height} scale={scale}"
            );
        }
    }

    #[test]
    fn scaling_moves_the_hotspot_with_the_image() {
        let source = Raster {
            width: 2,
            height: 2,
            xhot: 1,
            yhot: 1,
            pixels: vec![OPAQUE_WHITE; 4],
        };
        let scaled = source.scaled_to(8, 8).expect("scaled raster");
        assert_eq!((scaled.width, scaled.height), (8, 8));
        assert_eq!((scaled.xhot, scaled.yhot), (4, 4));
        assert!(scaled.pixels.iter().all(|pixel| *pixel == OPAQUE_WHITE));
    }
}
