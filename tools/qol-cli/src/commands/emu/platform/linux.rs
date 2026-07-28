use std::fs::OpenOptions;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

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

pub(crate) fn path_is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}
