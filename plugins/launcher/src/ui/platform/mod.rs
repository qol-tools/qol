#[cfg(not(target_os = "linux"))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;

#[cfg(not(target_os = "linux"))]
use fallback as imp;
#[cfg(target_os = "linux")]
use linux as imp;

pub(super) fn show_topmost_window(target_title: &str, all_titles: &[String]) {
    imp::show_topmost_window(target_title, all_titles);
}
