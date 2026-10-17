mod fallback;
mod linux;
mod macos;
mod windows;

use qol_platform::Os;

pub(super) fn link_section(target_os: Option<Os>) -> String {
    match target_os {
        Some(Os::Linux) => linux::link_section(),
        Some(Os::Macos) => macos::link_section(),
        Some(Os::Windows) => windows::link_section(),
        None => fallback::link_section(),
    }
}
