#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(super) use linux::{
    audio_server, process_running, read_autostart, remove_autostart, restart_service,
    service_journal, start_process, stop_process, supports_autostart, write_autostart,
};
#[cfg(target_os = "macos")]
pub(super) use macos::{
    audio_server, process_running, read_autostart, remove_autostart, restart_service,
    service_journal, start_process, stop_process, supports_autostart, write_autostart,
};
#[cfg(target_os = "windows")]
pub(super) use windows::{
    audio_server, process_running, read_autostart, remove_autostart, restart_service,
    service_journal, start_process, stop_process, supports_autostart, write_autostart,
};
