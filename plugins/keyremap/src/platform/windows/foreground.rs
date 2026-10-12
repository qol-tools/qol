use std::collections::HashSet;

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardLayout, HKL};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

use super::elevation;

const EXECUTABLE_SUFFIX: &str = ".exe";

#[derive(Debug)]
pub(super) struct Foreground {
    own: Option<bool>,
    process: (u32, Option<u64>),
    thread: u32,
    executable: String,
    injectable: bool,
}

impl Default for Foreground {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Foreground {
    pub(super) fn new(own: Option<bool>) -> Self {
        Self {
            own,
            process: (0, None),
            thread: 0,
            executable: String::new(),
            injectable: true,
        }
    }

    pub(super) fn refresh(&mut self) {
        let (pid, thread) = foreground_ids();
        self.thread = thread;
        let process = process_key(pid);
        if process != self.process {
            self.process = process;
            self.executable = image_name(pid).unwrap_or_default();
            self.injectable =
                pid == 0 || elevation::can_inject(self.own, elevation::of_process(pid));
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

fn foreground_ids() -> (u32, u32) {
    let window = unsafe { GetForegroundWindow() };
    if window.is_null() {
        return (0, 0);
    }
    let mut pid = 0u32;
    let thread = unsafe { GetWindowThreadProcessId(window, &mut pid) };
    (pid, thread)
}

fn process_key(pid: u32) -> (u32, Option<u64>) {
    if pid == 0 {
        return (0, None);
    }
    (pid, qol_process::process_start_time(pid))
}

fn image_name(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    let path = qol_process::process_image_path(pid).ok()?;
    Some(path.file_name()?.to_string_lossy().into_owned())
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
    fn the_image_name_is_the_executable_file_name() {
        let name = image_name(std::process::id()).expect("own image name");
        assert!(name.to_ascii_lowercase().ends_with(".exe"), "{name}");
        assert!(!name.contains('\\'), "{name}");
        assert_eq!(image_name(0), None);
    }

    #[test]
    fn a_process_is_keyed_by_its_pid_and_start_time() {
        let own = process_key(std::process::id());
        assert_eq!(own.0, std::process::id());
        assert!(own.1.is_some(), "{own:?}");
        assert_eq!(process_key(std::process::id()), own);
        assert_eq!(process_key(0), (0, None));
    }

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
