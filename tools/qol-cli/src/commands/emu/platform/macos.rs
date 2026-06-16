pub(crate) fn hypervisor() -> &'static str {
    "hvf"
}

pub(crate) fn hypervisor_available() -> bool {
    true
}

pub(crate) fn display() -> &'static str {
    "cocoa"
}

pub(crate) fn libvirt_uris() -> &'static [&'static str] {
    &[]
}
