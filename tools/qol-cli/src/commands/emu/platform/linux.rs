use std::path::Path;

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
