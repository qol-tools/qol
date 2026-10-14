#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "linux"))]
mod unsupported;

#[cfg(target_os = "linux")]
pub(crate) use linux::{
    boot_id, install, is_mint, missing, read_host, read_updates, restart_now, schedule_restart,
    set_auto, set_hours, supported, Helper,
};
#[cfg(not(target_os = "linux"))]
pub(crate) use unsupported::{
    boot_id, install, is_mint, missing, read_host, read_updates, restart_now, schedule_restart,
    set_auto, set_hours, supported, Helper,
};
