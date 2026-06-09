use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn acceleration() -> &'static str {
    if Path::new("/dev/kvm").exists() {
        "kvm"
    } else {
        "tcg"
    }
}

pub(crate) fn libvirt_uris() -> &'static [&'static str] {
    &["qemu:///system", "qemu:///session"]
}

pub(crate) fn image_search_roots(home: Option<PathBuf>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = home {
        roots.push(home.join("VMs"));
        roots.push(home.join("Virtual Machines"));
        roots.push(home.join(".local/share/gnome-boxes/images"));
    }
    if let Some(user) = env::var_os("USER").and_then(|user| user.into_string().ok()) {
        roots.extend(mounted_vm_roots(&PathBuf::from("/media").join(user)));
    }
    roots.extend(mounted_vm_roots(Path::new("/mnt")));
    roots
}

fn mounted_vm_roots(parent: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(parent) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .flat_map(|mount| [mount.join("VMs"), mount.join("Virtual Machines")])
        .collect()
}
