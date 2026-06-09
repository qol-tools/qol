use std::path::PathBuf;

pub(crate) fn acceleration() -> &'static str {
    "tcg"
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
