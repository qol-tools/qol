use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BootMedia {
    Disk,
    Iso,
}

impl BootMedia {
    pub(crate) fn from_path(path: &Path) -> BootMedia {
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("iso") => BootMedia::Iso,
            _ => BootMedia::Disk,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            BootMedia::Disk => "disk",
            BootMedia::Iso => "iso",
        }
    }

    pub(crate) fn requires_qemu_img(self) -> bool {
        matches!(self, BootMedia::Disk)
    }

    pub(crate) fn append_qemu_args(self, args: &mut Vec<String>, boot_media: &Path) {
        match self {
            BootMedia::Disk => args.extend([
                "-drive".to_string(),
                format!(
                    "file={},id=qoldisk,if=virtio,format=qcow2",
                    boot_media.display()
                ),
            ]),
            BootMedia::Iso => args.extend([
                "-boot".to_string(),
                "d".to_string(),
                "-cdrom".to_string(),
                boot_media.display().to_string(),
            ]),
        }
    }

    pub(crate) fn is_disk_image_path(path: &Path) -> bool {
        matches!(
            path.extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("qcow2" | "qcow" | "img" | "raw" | "vhd" | "vhdx" | "vmdk")
        )
    }
}

pub(crate) fn cleanup_artifacts(run_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    let entries =
        fs::read_dir(run_dir).with_context(|| format!("failed to read {}", run_dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && is_generated_artifact(&path) {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
            removed.push(path);
        }
    }
    removed.sort();
    Ok(removed)
}

fn is_generated_artifact(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name == "overlay.qcow2"
        || name == "usb-stick.raw"
        || (name.starts_with("overlay-snap-") && name.ends_with(".qcow2"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_path_maps_iso_extension_to_iso_else_disk() {
        let cases = [
            ("ubuntu.iso", BootMedia::Iso),
            ("linuxmint-22.1-cinnamon-64bit.iso", BootMedia::Iso),
            ("UPPER.ISO", BootMedia::Iso),
            ("disk.qcow2", BootMedia::Disk),
            ("disk.img", BootMedia::Disk),
            ("disk.vmdk", BootMedia::Disk),
            ("noext", BootMedia::Disk),
        ];
        for (name, expected) in cases {
            assert_eq!(
                BootMedia::from_path(Path::new(name)),
                expected,
                "name: {name}"
            );
        }
    }

    #[test]
    fn as_str_names_each_variant() {
        assert_eq!(BootMedia::Disk.as_str(), "disk");
        assert_eq!(BootMedia::Iso.as_str(), "iso");
    }

    #[test]
    fn qemu_img_is_required_only_for_disk_images() {
        assert!(BootMedia::Disk.requires_qemu_img());
        assert!(!BootMedia::Iso.requires_qemu_img());
    }

    #[test]
    fn append_qemu_args_maps_disk_and_iso_to_qemu_media() {
        let cases = [
            (
                BootMedia::Disk,
                "/tmp/disk.qcow2",
                vec![
                    "-drive",
                    "file=/tmp/disk.qcow2,id=qoldisk,if=virtio,format=qcow2",
                ],
            ),
            (
                BootMedia::Iso,
                "/tmp/installer.iso",
                vec!["-boot", "d", "-cdrom", "/tmp/installer.iso"],
            ),
        ];
        for (media, path, expected) in cases {
            let mut args = Vec::new();
            media.append_qemu_args(&mut args, Path::new(path));
            assert_eq!(
                args,
                expected.into_iter().map(str::to_string).collect::<Vec<_>>(),
                "media: {media:?}"
            );
        }
    }

    #[test]
    fn cleanup_artifacts_removes_generated_disk_files_not_iso_or_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let files = [
            "overlay.qcow2",
            "overlay-snap-1.qcow2",
            "usb-stick.raw",
            "manual.qcow2",
            "installer.iso",
            "report.json",
            "qemu-command.txt",
        ];
        for name in files {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }

        let removed = cleanup_artifacts(dir.path()).unwrap();

        assert_eq!(
            removed,
            vec![
                dir.path().join("overlay-snap-1.qcow2"),
                dir.path().join("overlay.qcow2"),
                dir.path().join("usb-stick.raw"),
            ]
        );
        assert!(dir.path().join("installer.iso").is_file());
        assert!(dir.path().join("manual.qcow2").is_file());
        assert!(dir.path().join("report.json").is_file());
        assert!(dir.path().join("qemu-command.txt").is_file());
    }
}
