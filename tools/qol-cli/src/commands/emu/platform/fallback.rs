use std::path::Path;

pub(crate) fn hypervisor() -> &'static str {
    "tcg"
}

pub(crate) fn hypervisor_available() -> bool {
    false
}

pub(crate) fn display() -> &'static str {
    "none"
}

pub(crate) fn libvirt_uris() -> &'static [&'static str] {
    &[]
}

pub(crate) fn path_is_executable(path: &Path) -> bool {
    path.is_file()
}
