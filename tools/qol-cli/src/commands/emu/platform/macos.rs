use std::path::PathBuf;

pub(crate) fn acceleration() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "hvf"
    } else {
        "tcg"
    }
}

pub(crate) fn display() -> &'static str {
    "cocoa"
}

pub(crate) fn libvirt_uris() -> &'static [&'static str] {
    &[]
}

pub(crate) fn image_search_roots(home: Option<PathBuf>) -> Vec<PathBuf> {
    let Some(home) = home else {
        return Vec::new();
    };
    vec![
        home.join("VMs"),
        home.join("Virtual Machines"),
        home.join("Library/Containers/com.utmapp.UTM/Data/Documents"),
    ]
}
