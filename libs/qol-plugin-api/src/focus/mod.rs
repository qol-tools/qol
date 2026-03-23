#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(target_os = "linux"))]
mod fallback;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(not(target_os = "linux"))]
use fallback as imp;

pub fn should_poll_process_focus() -> bool {
    imp::should_poll_process_focus()
}

pub fn has_process_focus() -> bool {
    imp::has_process_focus()
}
