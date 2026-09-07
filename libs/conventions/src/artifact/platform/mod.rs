mod fallback;
mod linux;
mod macos;
mod windows;

pub(super) fn link_section(target_os: &str) -> String {
    match target_os {
        "linux" => linux::link_section(),
        "macos" => macos::link_section(),
        "windows" => windows::link_section(),
        _ => fallback::link_section(),
    }
}
