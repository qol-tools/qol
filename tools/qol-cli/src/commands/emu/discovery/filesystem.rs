use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::super::arch::{infer_arch_from_filename, infer_firmware, GuestArch};
use super::super::{humanize_id, sanitize_id};
use super::candidate::ImageCandidate;

const MAX_SCAN_DEPTH: usize = 4;

pub(crate) fn collect_image_paths(roots: &[PathBuf], seen: &mut HashSet<PathBuf>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for root in roots {
        collect_into(root, MAX_SCAN_DEPTH, seen, &mut paths);
    }
    paths
}

pub(crate) fn legacy_root_image_count(registered: &HashSet<PathBuf>) -> usize {
    let roots = super::super::platform::image_search_roots(dirs::home_dir());
    count_unregistered(&roots, registered)
}

fn count_unregistered(roots: &[PathBuf], registered: &HashSet<PathBuf>) -> usize {
    let mut seen = HashSet::new();
    collect_image_paths(roots, &mut seen)
        .into_iter()
        .filter(|path| !registered.contains(path))
        .count()
}

fn collect_into(root: &Path, depth: usize, seen: &mut HashSet<PathBuf>, paths: &mut Vec<PathBuf>) {
    if depth == 0 || !root.is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_into(&path, depth - 1, seen, paths);
            continue;
        }
        if !is_vm_image_path(&path) {
            continue;
        }
        let canonical = path.canonicalize().unwrap_or(path);
        if seen.insert(canonical.clone()) {
            paths.push(canonical);
        }
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

#[allow(dead_code)]
pub(crate) fn infer_candidate(path: &Path) -> ImageCandidate {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let id = image_id(&canonical);
    let display_name = humanize_id(&id);
    let name = canonical
        .file_name()
        .and_then(|os| os.to_str())
        .unwrap_or_default();
    let inferred = infer_arch_from_filename(name);
    let arch = inferred.unwrap_or(GuestArch::X86_64);
    let firmware = infer_firmware(arch, name);
    ImageCandidate {
        id,
        path: canonical,
        display_name,
        arch,
        arch_inferred: inferred.is_some(),
        firmware,
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::arch::Firmware;
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

    #[test]
    fn collect_image_paths_walks_recursively_and_dedupes_non_images() {
        let root = std::env::temp_dir().join(format!("qol-emu-walk-{}", std::process::id()));
        let nested = root.join("sub");
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join("a.qcow2"), b"x").unwrap();
        fs::write(root.join("notes.txt"), b"x").unwrap();
        fs::write(nested.join("b.img"), b"x").unwrap();

        let mut seen = HashSet::new();
        let mut paths = collect_image_paths(std::slice::from_ref(&root), &mut seen);
        paths.sort();

        assert_eq!(paths.len(), 2, "paths: {paths:?}");
        assert!(
            paths.iter().any(|p| p.ends_with("a.qcow2")),
            "paths: {paths:?}"
        );
        assert!(
            paths.iter().any(|p| p.ends_with("b.img")),
            "paths: {paths:?}"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn infer_candidate_fills_arch_firmware_and_id_from_filename() {
        let root = std::env::temp_dir().join(format!("qol-emu-infer-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let image = root.join("win11-arm64.qcow2");
        fs::write(&image, b"x").unwrap();

        let candidate = infer_candidate(&image);

        assert_eq!(candidate.arch, GuestArch::Aarch64, "arm64 token");
        assert!(candidate.arch_inferred, "arch was inferred from filename");
        assert_eq!(candidate.firmware, Firmware::Uefi, "arm => uefi");
        assert_eq!(candidate.id, "win11-arm64");
        assert_eq!(candidate.path, image.canonicalize().unwrap());

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn legacy_count_excludes_registered_canonical_paths() {
        let root = std::env::temp_dir().join(format!("qol-emu-legacy-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.qcow2"), b"x").unwrap();
        fs::write(root.join("b.img"), b"x").unwrap();

        let mut seen = HashSet::new();
        let walked = collect_image_paths(std::slice::from_ref(&root), &mut seen);
        assert_eq!(walked.len(), 2, "walked: {walked:?}");

        let mut all = HashSet::new();
        assert_eq!(count_unregistered(std::slice::from_ref(&root), &all), 2);

        all.insert(walked[0].clone());
        assert_eq!(count_unregistered(std::slice::from_ref(&root), &all), 1);
        fs::remove_dir_all(&root).unwrap();
    }
}
