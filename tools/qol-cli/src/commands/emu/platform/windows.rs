use super::super::arch::GuestArch;

pub(crate) fn hypervisor() -> &'static str {
    "whpx"
}

pub(crate) fn hypervisor_available() -> bool {
    true
}

pub(crate) fn display() -> &'static str {
    "sdl"
}

pub(crate) fn libvirt_uris() -> &'static [&'static str] {
    &[]
}

pub(crate) fn supports_native_linux_payload(_guest: GuestArch) -> bool {
    false
}
