use std::fs::OpenOptions;

pub(crate) fn hypervisor() -> &'static str {
    "kvm"
}

pub(crate) fn hypervisor_available() -> bool {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .is_ok()
}

pub(crate) fn display() -> &'static str {
    "gtk,zoom-to-fit=on"
}

pub(crate) fn libvirt_uris() -> &'static [&'static str] {
    &["qemu:///system", "qemu:///session"]
}
