use std::path::PathBuf;

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

pub(crate) fn image_search_roots(home: Option<PathBuf>) -> Vec<PathBuf> {
    let Some(home) = home else {
        return Vec::new();
    };
    vec![home.join("VMs"), home.join("Virtual Machines")]
}
