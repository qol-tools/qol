use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::super::{humanize_id, sanitize_id, Environment};

const MAX_SCAN_DEPTH: usize = 4;

pub(crate) fn discover(roots: &[PathBuf]) -> Vec<Environment> {
    let mut environments = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        collect_image_environments(root, MAX_SCAN_DEPTH, &mut seen, &mut environments);
    }
    environments
}

fn collect_image_environments(
    root: &Path,
    depth: usize,
    seen: &mut HashSet<PathBuf>,
    environments: &mut Vec<Environment>,
) {
    if depth == 0 || !root.is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_image_environments(&path, depth - 1, seen, environments);
            continue;
        }
        if !is_vm_image_path(&path) {
            continue;
        }
        let canonical = path.canonicalize().unwrap_or(path);
        if !seen.insert(canonical.clone()) {
            continue;
        }
        let id = image_id(&canonical);
        environments.push(Environment {
            name: humanize_id(&id),
            id,
            backend: "qemu".to_string(),
            arch: "x86_64".to_string(),
            image_path: canonical,
            source: "scan".to_string(),
        });
    }
}

pub(crate) fn is_vm_image_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("qcow2" | "qcow" | "img" | "raw" | "vhd" | "vhdx" | "vmdk")
    )
}

pub(crate) fn image_id(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("emu");
    let source = if matches!(
        stem.to_ascii_lowercase().as_str(),
        "disk" | "drive" | "image" | "hda"
    ) {
        path.parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or(stem)
    } else {
        stem
    };
    sanitize_id(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_id_uses_parent_for_generic_disk_names() {
        assert_eq!(
            image_id(Path::new("/vm/windows-11/disk.qcow2")),
            "windows-11"
        );
        assert_eq!(image_id(Path::new("/vm/win.qcow2")), "win");
    }

    #[test]
    fn recognizes_vm_image_extensions() {
        assert!(is_vm_image_path(Path::new("a.qcow2")));
        assert!(is_vm_image_path(Path::new("a.VHDX")));
        assert!(!is_vm_image_path(Path::new("a.txt")));
    }
}
