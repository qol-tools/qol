use anyhow::{anyhow, Context, Result};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::null_mut;
use std::time::Duration;
use windows_sys::Win32::Foundation::GlobalFree;
use windows_sys::Win32::Graphics::Gdi::BI_RGB;
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::System::Ole::{CF_DIB, CF_UNICODETEXT};

const OPEN_ATTEMPTS: u32 = 10;
const OPEN_RETRY_DELAY: Duration = Duration::from_millis(25);
const BITMAP_HEADER_BYTES: u32 = 40;
const BITMAP_PLANES: u16 = 1;
const BITMAP_BITS_PER_PIXEL: u16 = 32;
const PNG_FORMAT_NAME: &str = "PNG";

pub fn copy_image_to_clipboard(path: &Path) -> Result<()> {
    let png = std::fs::read(path)
        .with_context(|| format!("failed to read screenshot: {}", path.display()))?;
    let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
        .with_context(|| format!("failed to decode screenshot: {}", path.display()))?
        .to_rgba8();
    let dib = dib_from_rgba(image.as_raw(), image.width(), image.height())
        .ok_or_else(|| anyhow!("screenshot is too large for a clipboard bitmap"))?;
    let png_format = registered_format(PNG_FORMAT_NAME)?;
    write_clipboard(&[(u32::from(CF_DIB), &dib), (png_format, &png)])
}

pub fn copy_path_to_clipboard(path: &Path) -> Result<()> {
    let text = utf16_text_bytes(path);
    write_clipboard(&[(u32::from(CF_UNICODETEXT), &text)])
}

fn utf16_text_bytes(path: &Path) -> Vec<u8> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn wide_name(name: &str) -> Vec<u16> {
    name.encode_utf16().chain(std::iter::once(0)).collect()
}

fn registered_format(name: &str) -> Result<u32> {
    let wide = wide_name(name);
    let format = unsafe { RegisterClipboardFormatW(wide.as_ptr()) };
    if format == 0 {
        return Err(anyhow!(
            "failed to register the {name} clipboard format: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(format)
}

fn dib_from_rgba(rgba: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    if width == 0 || height == 0 {
        return None;
    }
    let row_bytes = usize::try_from(width).ok()?.checked_mul(4)?;
    let image_bytes = row_bytes.checked_mul(usize::try_from(height).ok()?)?;
    if rgba.len() != image_bytes {
        return None;
    }
    let mut dib = bitmap_header(
        i32::try_from(width).ok()?,
        i32::try_from(height).ok()?,
        u32::try_from(image_bytes).ok()?,
    );
    dib.reserve(image_bytes);
    for row in rgba.chunks_exact(row_bytes).rev() {
        for pixel in row.chunks_exact(4) {
            dib.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    Some(dib)
}

fn bitmap_header(width: i32, height: i32, image_bytes: u32) -> Vec<u8> {
    let unused_resolution = 0i32;
    let unused_palette = 0u32;
    [
        BITMAP_HEADER_BYTES.to_le_bytes().as_slice(),
        &width.to_le_bytes(),
        &height.to_le_bytes(),
        &BITMAP_PLANES.to_le_bytes(),
        &BITMAP_BITS_PER_PIXEL.to_le_bytes(),
        &BI_RGB.to_le_bytes(),
        &image_bytes.to_le_bytes(),
        &unused_resolution.to_le_bytes(),
        &unused_resolution.to_le_bytes(),
        &unused_palette.to_le_bytes(),
        &unused_palette.to_le_bytes(),
    ]
    .concat()
}

fn write_clipboard(entries: &[(u32, &[u8])]) -> Result<()> {
    let _clipboard = OpenClipboardGuard::open()?;
    if unsafe { EmptyClipboard() } == 0 {
        return Err(anyhow!(
            "failed to empty the clipboard: {}",
            std::io::Error::last_os_error()
        ));
    }
    for (format, bytes) in entries {
        set_clipboard_bytes(*format, bytes)?;
    }
    Ok(())
}

struct OpenClipboardGuard;

impl OpenClipboardGuard {
    fn open() -> Result<Self> {
        for _ in 0..OPEN_ATTEMPTS {
            if unsafe { OpenClipboard(null_mut()) } != 0 {
                return Ok(Self);
            }
            std::thread::sleep(OPEN_RETRY_DELAY);
        }
        Err(anyhow!(
            "failed to open the clipboard: {}",
            std::io::Error::last_os_error()
        ))
    }
}

impl Drop for OpenClipboardGuard {
    fn drop(&mut self) {
        unsafe { CloseClipboard() };
    }
}

fn set_clipboard_bytes(format: u32, bytes: &[u8]) -> Result<()> {
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) };
    if memory.is_null() {
        return Err(anyhow!(
            "failed to allocate {} clipboard bytes: {}",
            bytes.len(),
            std::io::Error::last_os_error()
        ));
    }
    let target = unsafe { GlobalLock(memory) };
    if target.is_null() {
        let error = std::io::Error::last_os_error();
        unsafe { GlobalFree(memory) };
        return Err(anyhow!("failed to lock clipboard memory: {error}"));
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), target.cast::<u8>(), bytes.len());
        GlobalUnlock(memory);
    }
    if unsafe { SetClipboardData(format, memory) }.is_null() {
        let error = std::io::Error::last_os_error();
        unsafe { GlobalFree(memory) };
        return Err(anyhow!(
            "failed to place clipboard format {format}: {error}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{bitmap_header, dib_from_rgba, utf16_text_bytes, wide_name};
    use std::path::Path;

    #[test]
    fn bitmap_header_describes_a_bottom_up_32_bit_dib() {
        let header = bitmap_header(3, 2, 24);
        let field = |offset: usize, width: usize| header[offset..offset + width].to_vec();
        let cases: [(usize, Vec<u8>); 11] = [
            (0, 40u32.to_le_bytes().to_vec()),
            (4, 3i32.to_le_bytes().to_vec()),
            (8, 2i32.to_le_bytes().to_vec()),
            (12, 1u16.to_le_bytes().to_vec()),
            (14, 32u16.to_le_bytes().to_vec()),
            (16, 0u32.to_le_bytes().to_vec()),
            (20, 24u32.to_le_bytes().to_vec()),
            (24, 0i32.to_le_bytes().to_vec()),
            (28, 0i32.to_le_bytes().to_vec()),
            (32, 0u32.to_le_bytes().to_vec()),
            (36, 0u32.to_le_bytes().to_vec()),
        ];
        assert_eq!(header.len(), 40);
        for (offset, expected) in cases {
            assert_eq!(field(offset, expected.len()), expected, "offset {offset}");
        }
    }

    #[test]
    fn dib_pixels_flip_rows_and_swap_to_bgra() {
        let top = [1, 2, 3, 4, 5, 6, 7, 8];
        let bottom = [9, 10, 11, 12, 13, 14, 15, 16];
        let two_rows = [top, bottom].concat();
        let cases = [
            (
                "single pixel",
                vec![10, 20, 30, 255],
                1,
                1,
                Some(vec![30, 20, 10, 255]),
            ),
            (
                "two rows",
                two_rows,
                2,
                2,
                Some(vec![11, 10, 9, 12, 15, 14, 13, 16, 3, 2, 1, 4, 7, 6, 5, 8]),
            ),
            ("empty width", Vec::new(), 0, 1, None),
            ("empty height", Vec::new(), 1, 0, None),
            ("short buffer", vec![1, 2, 3], 1, 1, None),
        ];
        for (name, rgba, width, height, expected) in cases {
            let pixels = dib_from_rgba(&rgba, width, height).map(|dib| dib[40..].to_vec());
            assert_eq!(pixels, expected, "{name}");
        }
    }

    #[test]
    fn clipboard_text_is_nul_terminated_utf16() {
        let cases = [
            (Path::new("C:\\a.png"), "C:\\a.png"),
            (Path::new("C:\\Bilder\\skærm.png"), "C:\\Bilder\\skærm.png"),
        ];
        for (path, text) in cases {
            let expected: Vec<u8> = wide_name(text)
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect();
            assert_eq!(utf16_text_bytes(path), expected, "{text}");
        }
    }
}
