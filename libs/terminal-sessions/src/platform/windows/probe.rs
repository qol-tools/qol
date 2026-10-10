use std::io;

use windows_sys::Win32::System::Console::{
    GetConsoleProcessList, GetConsoleScreenBufferInfo, GetConsoleTitleW, GetConsoleWindow,
    ReadConsoleOutputCharacterW, CONSOLE_SCREEN_BUFFER_INFO, COORD,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindow, IsWindowVisible, GW_OWNER};

use super::attach::{ConsoleFile, Detached};
use super::peb::process_strings;
use super::report::{live_rows, ConsoleReport, ProcessDetail};

const MAX_ATTACHED: usize = 256;
const TITLE_CAPACITY: usize = 1024;

pub(super) fn run(args: &[String]) -> io::Result<String> {
    let pids = args
        .iter()
        .map(|arg| {
            arg.parse::<u32>().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("invalid process id {arg:?}"),
                )
            })
        })
        .collect::<io::Result<Vec<u32>>>()?;
    let scope = Detached::begin();
    let reports: Vec<ConsoleReport> = pids
        .into_iter()
        .filter_map(|pid| {
            let _attached = scope.attach(pid).ok()?;
            read_attached(pid)
        })
        .collect();
    drop(scope);
    serde_json::to_string(&reports).map_err(io::Error::other)
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
    let output = ConsoleFile::open("CONOUT$").ok()?;
    let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
    if unsafe { GetConsoleScreenBufferInfo(output.0, &mut info) } == 0 {
        return None;
    }
    let view = info.srWindow;
    let width = u32::try_from(i32::from(view.Right) - i32::from(view.Left) + 1).ok()?;
    let live_screen = live_rows(view.Top, view.Bottom, info.dwCursorPosition.Y)
        .map(|(top, bottom)| rows(&output, view.Left, top, bottom, width));
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
        window_owned: !window.is_null() && !unsafe { GetWindow(window, GW_OWNER) }.is_null(),
        screen: rows(&output, view.Left, view.Top, view.Bottom, width),
        live_screen,
        processes,
    })
}

fn rows(output: &ConsoleFile, left: i16, top: i16, bottom: i16, width: u32) -> String {
    (top..=bottom)
        .map(|row| read_row(output, left, row, width))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

fn read_row(output: &ConsoleFile, left: i16, row: i16, width: u32) -> String {
    let mut buffer = vec![0u16; width as usize];
    let mut read = 0u32;
    let ok = unsafe {
        ReadConsoleOutputCharacterW(
            output.0,
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
