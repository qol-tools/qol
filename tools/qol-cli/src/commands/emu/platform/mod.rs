#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::*;
#[cfg(target_os = "macos")]
pub(crate) use macos::*;
#[cfg(target_os = "windows")]
pub(crate) use windows::*;

use super::arch::GuestArch;

pub(crate) fn acceleration(guest: GuestArch) -> &'static str {
    select(
        hypervisor(),
        hypervisor_available(),
        std::env::consts::ARCH,
        guest,
    )
}

fn select(
    hypervisor: &'static str,
    available: bool,
    host_arch: &str,
    guest: GuestArch,
) -> &'static str {
    if available && host_arch == guest.as_str() {
        hypervisor
    } else {
        "tcg"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_accelerates_only_matching_available_arch() {
        let cases = [
            ("hvf", true, "aarch64", GuestArch::Aarch64, "hvf"),
            ("hvf", true, "aarch64", GuestArch::X86_64, "tcg"),
            ("kvm", false, "x86_64", GuestArch::X86_64, "tcg"),
            ("whpx", true, "x86_64", GuestArch::X86_64, "whpx"),
        ];
        for (hypervisor, available, host, guest, expected) in cases {
            assert_eq!(
                select(hypervisor, available, host, guest),
                expected,
                "hypervisor: {hypervisor}, host: {host}, guest: {guest:?}"
            );
        }
    }
}
