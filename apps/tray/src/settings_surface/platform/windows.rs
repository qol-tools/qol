#[path = "gpui_host.rs"]
mod gpui_host;
#[path = "native_tools/mod.rs"]
mod native_tools;

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

pub(in crate::settings_surface) use gpui_host::{
    apply_theme, plugins_changed, prewarm, request, run, show_toast, stop, wait_until_ready,
};

pub(in crate::settings_surface) fn native_available() -> bool {
    true
}

const FILETIME_TICKS_PER_MS: u64 = 10_000;
const FILETIME_UNIX_EPOCH_TICKS: u64 = 116_444_736_000_000_000;

fn process_elapsed_ms() -> Option<u64> {
    let mut creation: FILETIME = unsafe { std::mem::zeroed() };
    let mut exit: FILETIME = unsafe { std::mem::zeroed() };
    let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
    let mut user: FILETIME = unsafe { std::mem::zeroed() };
    let read = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if read == 0 {
        return None;
    }
    let ticks = (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
    let started_ms = ticks.checked_sub(FILETIME_UNIX_EPOCH_TICKS)? / FILETIME_TICKS_PER_MS;
    let started =
        std::time::UNIX_EPOCH.checked_add(std::time::Duration::from_millis(started_ms))?;
    std::time::SystemTime::now()
        .duration_since(started)
        .ok()
        .map(|elapsed| elapsed.as_millis() as u64)
}
