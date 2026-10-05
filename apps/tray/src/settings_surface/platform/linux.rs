#[path = "native_tools/mod.rs"]
mod native_tools;
#[path = "unix_common.rs"]
mod unix_common;

pub(in crate::settings_surface) use unix_common::{
    apply_theme, plugins_changed, prewarm, request, run, show_toast, stop, wait_until_ready,
};

pub(in crate::settings_surface) fn native_available() -> bool {
    true
}

const USER_HZ: u64 = 100;

fn process_elapsed_ms() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let start_ticks = stat
        .rsplit_once(')')?
        .1
        .split_ascii_whitespace()
        .nth(19)?
        .parse::<u64>()
        .ok()?;
    let uptime = std::fs::read_to_string("/proc/uptime").ok()?;
    let uptime_ms = (uptime
        .split_ascii_whitespace()
        .next()?
        .parse::<f64>()
        .ok()?
        * 1000.0) as u64;
    uptime_ms.checked_sub(start_ticks * 1000 / USER_HZ)
}
