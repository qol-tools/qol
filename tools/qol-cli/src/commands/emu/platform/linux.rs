use std::path::Path;

use super::super::arch::GuestArch;

pub(crate) fn hypervisor() -> &'static str {
    "kvm"
}

pub(crate) fn hypervisor_available() -> bool {
    Path::new("/dev/kvm").exists()
}

pub(crate) fn display() -> &'static str {
    "gtk,zoom-to-fit=on"
}

pub(crate) fn libvirt_uris() -> &'static [&'static str] {
    &["qemu:///system", "qemu:///session"]
}

pub(crate) fn supports_native_linux_payload(guest: GuestArch) -> bool {
    std::env::consts::ARCH == guest.as_str()
}
