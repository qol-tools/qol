use std::collections::HashSet;

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardLayout, HKL};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use super::elevation;

const EXECUTABLE_SUFFIX: &str = ".exe";

#[derive(Debug)]
pub(super) struct Foreground {
    pid: u32,
    thread: u32,
    executable: String,
    injectable: bool,
}

impl Default for Foreground {
    fn default() -> Self {
        Self {
            pid: 0,
            thread: 0,
            executable: String::new(),
            injectable: true,
        }
    }
}

impl Foreground {
    pub(super) fn refresh(&mut self) {
        let window = unsafe { GetForegroundWindow() };
        let mut pid = 0u32;
        self.thread = if window.is_null() {
            0
        } else {
            unsafe { GetWindowThreadProcessId(window, &mut pid) }
        };
        if pid != self.pid {
            self.pid = pid;
            self.executable = executable_name(pid).unwrap_or_default();
            self.injectable =
                pid == 0 || elevation::can_inject(elevation::current(), elevation::of_process(pid));
        }
    }

    pub(super) fn app(&self, excluded: &HashSet<String>) -> String {
        app_id(&self.executable, excluded)
    }

    pub(super) fn injectable(&self) -> bool {
        self.injectable
    }

    pub(super) fn layout(&self) -> HKL {
        unsafe { GetKeyboardLayout(self.thread) }
    }
}

fn executable_name(pid: u32) -> Option<String> {
    let pid = i32::try_from(pid).ok().filter(|pid| *pid > 0)?;
    qol_app_icon::processes()
        .into_iter()
        .find(|entry| entry.pid == pid)
        .map(|entry| entry.name)
}

pub(super) fn app_id(executable: &str, excluded: &HashSet<String>) -> String {
    let stem = strip_suffix(executable);
    excluded
        .iter()
        .find(|entry| {
            let entry = entry.trim();
            !entry.is_empty()
                && (entry.eq_ignore_ascii_case(executable)
                    || strip_suffix(entry).eq_ignore_ascii_case(stem))
        })
        .cloned()
        .unwrap_or_else(|| executable.to_string())
}

fn strip_suffix(name: &str) -> &str {
    let split = name.len().saturating_sub(EXECUTABLE_SUFFIX.len());
    match (name.get(..split), name.get(split..)) {
        (Some(stem), Some(suffix)) if suffix.eq_ignore_ascii_case(EXECUTABLE_SUFFIX) => stem,
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excluded_apps_match_the_executable_loosely() {
        let excluded: HashSet<String> = ["Code.exe", "WindowsTerminal", "com.apple.Terminal", ""]
            .into_iter()
            .map(String::from)
            .collect();
        let cases = [
            ("Code.exe", "Code.exe"),
            ("code.EXE", "Code.exe"),
            ("WindowsTerminal.exe", "WindowsTerminal"),
            ("windowsterminal.exe", "WindowsTerminal"),
            ("notepad.exe", "notepad.exe"),
            ("Terminal.exe", "Terminal.exe"),
            ("", ""),
        ];
        for (executable, expected) in cases {
            assert_eq!(app_id(executable, &excluded), expected, "{executable}");
        }
    }

    #[test]
    fn only_a_trailing_exe_is_stripped() {
        let cases = [
            ("notepad.exe", "notepad"),
            ("NOTEPAD.EXE", "NOTEPAD"),
            ("exe", "exe"),
            (".exe", ""),
            ("my.exe.tool", "my.exe.tool"),
            ("été.exe", "été"),
        ];
        for (name, expected) in cases {
            assert_eq!(strip_suffix(name), expected, "{name}");
        }
    }
}
