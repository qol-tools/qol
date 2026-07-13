use super::super::arch::GuestArch;

pub(crate) fn hypervisor() -> &'static str {
    "hvf"
}

pub(crate) fn hypervisor_available() -> bool {
    true
}

pub(crate) fn display() -> &'static str {
    "cocoa,zoom-to-fit=on"
}

pub(crate) fn libvirt_uris() -> &'static [&'static str] {
    &[]
}

pub(crate) fn supports_native_linux_payload(_guest: GuestArch) -> bool {
    false
}
