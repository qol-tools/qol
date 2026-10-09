use std::ffi::c_void;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Shell::SHDefExtractIconW;
use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, DrawIconEx, DI_NORMAL, HICON};

use crate::{ProcessEntry, RgbaImage};

use super::AppIconPlatform;

const UNIX_EPOCH_AS_FILETIME_US: u64 = 11_644_473_600_000_000;

pub(super) struct Platform;

impl AppIconPlatform for Platform {
    fn icon_for_bundle_id(&self, _bundle_id: &str, _size: usize) -> Option<RgbaImage> {
        None
    }

    fn icon_png_for_path(&self, _path: &Path, _size: usize) -> Option<Vec<u8>> {
        None
    }

    fn icon_png_for_bundle_id(&self, _bundle_id: &str, _size: usize) -> Option<Vec<u8>> {
        None
    }

    fn icon_for_pid(&self, pid: i32, size: usize) -> Option<RgbaImage> {
        let path = executable_path(u32::try_from(pid).ok()?)?;
        let icon = extract_icon(&path, size)?;
        let image = render_icon(icon, size);
        unsafe { DestroyIcon(icon) };
        image
    }

    fn app_display_name(&self, app_id: &str) -> Option<String> {
        (!app_id.is_empty()).then(|| app_id.to_string())
    }

    fn parent_pid(&self, pid: i32) -> Option<i32> {
        let pid = u32::try_from(pid).ok()?;
        let parent = process_entries()
            .into_iter()
            .find(|entry| entry.th32ProcessID == pid)?
            .th32ParentProcessID;
        i32::try_from(parent).ok()
    }

    fn process_executable(&self, pid: i32) -> Option<PathBuf> {
        let mut path = executable_path(u32::try_from(pid).ok()?)?;
        path.pop();
        Some(PathBuf::from(OsString::from_wide(&path)))
    }

    fn processes(&self) -> Vec<ProcessEntry> {
        process_entries()
            .iter()
            .filter_map(|entry| {
                Some(ProcessEntry {
                    pid: i32::try_from(entry.th32ProcessID).ok()?,
                    parent_pid: i32::try_from(entry.th32ParentProcessID).ok()?,
                    name: wide_name(&entry.szExeFile),
                })
            })
            .collect()
    }

    fn process_start_time_us(&self, pid: i32) -> Option<u64> {
        let process = open_process(u32::try_from(pid).ok()?)?;
        let mut times = [empty_filetime(); 4];
        let [created, exited, kernel, user] = &mut times;
        let read = unsafe { GetProcessTimes(process, created, exited, kernel, user) };
        unsafe { CloseHandle(process) };
        if read == 0 {
            return None;
        }
        let since_1601_us =
            ((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime)) / 10;
        since_1601_us.checked_sub(UNIX_EPOCH_AS_FILETIME_US)
    }
}

fn open_process(pid: u32) -> Option<HANDLE> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    (!process.is_null()).then_some(process)
}

fn executable_path(pid: u32) -> Option<Vec<u16>> {
    let process = open_process(pid)?;
    let mut buffer = vec![0u16; 1024];
    let mut length = buffer.len() as u32;
    let read = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            buffer.as_mut_ptr(),
            &mut length,
        )
    };
    unsafe { CloseHandle(process) };
    if read == 0 {
        return None;
    }
    buffer.truncate(length as usize);
    buffer.push(0);
    Some(buffer)
}

fn extract_icon(path: &[u16], size: usize) -> Option<HICON> {
    let mut icon: HICON = null_mut();
    let result =
        unsafe { SHDefExtractIconW(path.as_ptr(), 0, 0, &mut icon, null_mut(), size as u32) };
    (result >= 0 && !icon.is_null()).then_some(icon)
}

fn render_icon(icon: HICON, size: usize) -> Option<RgbaImage> {
    let side = i32::try_from(size).ok()?;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: side,
            biHeight: -side,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        bmiColors: [unsafe { std::mem::zeroed() }],
    };
    let dc = unsafe { CreateCompatibleDC(null_mut()) };
    if dc.is_null() {
        return None;
    }
    let mut bits: *mut c_void = null_mut();
    let bitmap = unsafe { CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0) };
    if bitmap.is_null() || bits.is_null() {
        unsafe { DeleteDC(dc) };
        return None;
    }
    let previous = unsafe { SelectObject(dc, bitmap) };
    let drawn = unsafe { DrawIconEx(dc, 0, 0, icon, side, side, 0, null_mut(), DI_NORMAL) } != 0;
    let length = size * size * 4;
    let bgra = unsafe { std::slice::from_raw_parts(bits as *const u8, length) }.to_vec();
    unsafe {
        SelectObject(dc, previous);
        DeleteObject(bitmap);
        DeleteDC(dc);
    }
    drawn.then(|| RgbaImage {
        data: rgba_from_bgra(bgra),
        width: size,
        height: size,
    })
}

fn rgba_from_bgra(mut pixels: Vec<u8>) -> Vec<u8> {
    let has_alpha = pixels.chunks_exact(4).any(|pixel| pixel[3] != 0);
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        if !has_alpha {
            pixel[3] = 255;
        }
    }
    pixels
}

fn process_entries() -> Vec<PROCESSENTRY32W> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Vec::new();
    }
    let mut entries = Vec::new();
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while more {
        entries.push(entry);
        more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot) };
    entries
}

fn wide_name(wide: &[u16]) -> String {
    let end = wide
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(wide.len());
    String::from_utf16_lossy(&wide[..end])
}

fn empty_filetime() -> FILETIME {
    FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{rgba_from_bgra, wide_name};

    #[test]
    fn wide_name_stops_at_the_first_nul() {
        let cases = [
            ("cmd.exe\0\0junk", "cmd.exe"),
            ("pwsh.exe", "pwsh.exe"),
            ("", ""),
        ];
        for (raw, expected) in cases {
            let wide: Vec<u16> = raw.encode_utf16().collect();
            assert_eq!(wide_name(&wide), expected, "{raw:?}");
        }
    }

    #[test]
    fn bgra_pixels_become_rgba() {
        let cases = [
            (
                "keeps alpha",
                vec![1, 2, 3, 128, 4, 5, 6, 0],
                vec![3, 2, 1, 128, 6, 5, 4, 0],
            ),
            ("opaque without alpha", vec![1, 2, 3, 0], vec![3, 2, 1, 255]),
        ];
        for (name, bgra, rgba) in cases {
            assert_eq!(rgba_from_bgra(bgra), rgba, "{name}");
        }
    }
}
