use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};

use anyhow::{Context, Result};
use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, TRUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Console::{
    AttachConsole, FreeConsole, GetConsoleProcessList, GetConsoleScreenBufferInfo,
    GetConsoleTitleW, GetConsoleWindow, GetStdHandle, ReadConsoleOutputCharacterW,
    SetConsoleCtrlHandler, SetStdHandle, CONSOLE_SCREEN_BUFFER_INFO, COORD, STD_ERROR_HANDLE,
    STD_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::IsWindowVisible;

use super::peb::process_strings;
use super::report::{ConsoleReport, ProcessDetail};

const STD_HANDLES: [STD_HANDLE; 3] = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE];
const MAX_ATTACHED: usize = 256;
const TITLE_CAPACITY: usize = 1024;

pub(super) fn run(args: &[String]) -> Result<String> {
    let pids = args
        .iter()
        .map(|arg| {
            arg.parse::<u32>()
                .with_context(|| format!("invalid process id {arg:?}"))
        })
        .collect::<Result<Vec<u32>>>()?;
    let saved = STD_HANDLES.map(|kind| unsafe { GetStdHandle(kind) });
    unsafe { SetConsoleCtrlHandler(None, TRUE) };
    let reports: Vec<ConsoleReport> = pids.into_iter().filter_map(inspect).collect();
    unsafe { FreeConsole() };
    for (kind, handle) in STD_HANDLES.into_iter().zip(saved) {
        unsafe { SetStdHandle(kind, handle) };
    }
    serde_json::to_string(&reports).context("could not encode console reports")
}

fn inspect(pid: u32) -> Option<ConsoleReport> {
    unsafe { FreeConsole() };
    if unsafe { AttachConsole(pid) } == 0 {
        return None;
    }
    let report = read_attached(pid);
    unsafe { FreeConsole() };
    report
}

fn read_attached(root: u32) -> Option<ConsoleReport> {
    let own = std::process::id();
    let mut list = [0u32; MAX_ATTACHED];
    let count = unsafe { GetConsoleProcessList(list.as_mut_ptr(), list.len() as u32) } as usize;
    let attached: Vec<i32> = list[..count.min(list.len())]
        .iter()
        .filter(|pid| **pid != own)
        .filter_map(|pid| i32::try_from(*pid).ok())
        .collect();
    let mut title = [0u16; TITLE_CAPACITY];
    let title_length = unsafe { GetConsoleTitleW(title.as_mut_ptr(), title.len() as u32) } as usize;
    let window = unsafe { GetConsoleWindow() };
    let output = Output::open()?;
    let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
    if unsafe { GetConsoleScreenBufferInfo(output.0, &mut info) } == 0 {
        return None;
    }
    let view = info.srWindow;
    let width = u32::try_from(i32::from(view.Right) - i32::from(view.Left) + 1).ok()?;
    let rows: Vec<String> = (view.Top..=view.Bottom)
        .map(|row| output.row(view.Left, row, width))
        .collect();
    let cursor = info.dwCursorPosition.Y;
    let processes = attached
        .iter()
        .map(|pid| {
            let strings = process_strings(*pid);
            ProcessDetail {
                pid: *pid,
                cwd: strings.cwd,
                command_line: strings.command_line,
            }
        })
        .collect();
    Some(ConsoleReport {
        root: i32::try_from(root).ok()?,
        attached,
        title: String::from_utf16_lossy(&title[..title_length.min(title.len())]),
        window: window as usize as u64,
        window_visible: !window.is_null() && unsafe { IsWindowVisible(window) } != 0,
        screen: rows.join("\n").trim_end().to_string(),
        viewport_current: (view.Top..=view.Bottom).contains(&cursor),
        processes,
    })
}

struct Output(HANDLE);

impl Drop for Output {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

impl Output {
    fn open() -> Option<Output> {
        let name: Vec<u16> = OsStr::new("CONOUT$").encode_wide().chain(Some(0)).collect();
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                null(),
                OPEN_EXISTING,
                0,
                null_mut(),
            )
        };
        (handle != INVALID_HANDLE_VALUE && !handle.is_null()).then_some(Output(handle))
    }

    fn row(&self, left: i16, row: i16, width: u32) -> String {
        let mut buffer = vec![0u16; width as usize];
        let mut read = 0u32;
        let ok = unsafe {
            ReadConsoleOutputCharacterW(
                self.0,
                buffer.as_mut_ptr(),
                width,
                COORD { X: left, Y: row },
                &mut read,
            )
        };
        if ok == 0 {
            return String::new();
        }
        buffer.truncate(read as usize);
        String::from_utf16_lossy(&buffer).trim_end().to_string()
    }
}
