use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use toml_edit::{value, DocumentMut, Item, Table};

use super::discovery::{parse_image_overrides, ImageCandidate};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct QemuImgInfo {
    pub(crate) format: String,
    pub(crate) virtual_size: u64,
}

const KNOWN_FORMATS: &[&str] = &["qcow2", "qcow", "raw", "vhd", "vhdx", "vmdk", "vpc"];

#[allow(dead_code)]
pub(crate) fn register_image(
    emu_toml: &Path,
    candidate: &ImageCandidate,
    qemu_img: &Path,
) -> Result<String> {
    let path = candidate.path.to_str().ok_or_else(|| {
        anyhow!(
            "image path is not valid UTF-8: {}",
            candidate.path.display()
        )
    })?;
    let output = Command::new(qemu_img)
        .args(["info", "--output=json", path])
        .output()
        .with_context(|| format!("failed to run {}", qemu_img.display()))?;
    if !output.status.success() {
        return Err(anyhow!(
            "qemu-img info failed for {}: {}",
            candidate.path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let _info = parse_qemu_img_info(&stdout)?;
    write_image_entry(emu_toml, candidate)
}

fn canonical_or_self(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn write_image_entry(emu_toml: &Path, candidate: &ImageCandidate) -> Result<String> {
    let existing = match std::fs::read_to_string(emu_toml) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", emu_toml.display()))
        }
    };
    let overrides = parse_image_overrides(&existing, None)
        .with_context(|| format!("failed to parse {}", emu_toml.display()))?;
    let registered: HashSet<PathBuf> = overrides
        .values()
        .map(|(path, _, _)| canonical_or_self(path))
        .collect();
    let candidate_canonical = canonical_or_self(&candidate.path);
    if registered.contains(&candidate_canonical) || overrides.contains_key(&candidate.id) {
        return Ok(candidate.id.clone());
    }
    let mut document = existing
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse {}", emu_toml.display()))?;
    let mut table = Table::new();
    table.insert("path", value(candidate.path.to_string_lossy().into_owned()));
    table.insert("arch", value(candidate.arch.as_str()));
    table.insert("firmware", value(candidate.firmware.as_str()));
    let images = document
        .entry("images")
        .or_insert(Item::Table(Table::new()))
        .as_table_mut()
        .ok_or_else(|| anyhow!("`images` in {} is not a table", emu_toml.display()))?;
    images.insert(&candidate.id, Item::Table(table));
    std::fs::write(emu_toml, document.to_string())
        .with_context(|| format!("failed to write {}", emu_toml.display()))?;
    Ok(candidate.id.clone())
}

pub(crate) fn parse_qemu_img_info(json: &str) -> Result<QemuImgInfo> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| anyhow!("invalid qemu-img JSON: {e}"))?;
    let format = value
        .get("format")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("qemu-img info missing `format`"))?
        .to_string();
    if !KNOWN_FORMATS.contains(&format.as_str()) {
        return Err(anyhow!("unknown image format `{format}`"));
    }
    let virtual_size = value
        .get("virtual-size")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| anyhow!("qemu-img info missing `virtual-size`"))?;
    Ok(QemuImgInfo {
        format,
        virtual_size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_qemu_img_info_json() {
        let json = r#"{"virtual-size":21474836480,"filename":"/a/b/x.qcow2","format":"qcow2","actual-size":1234}"#;
        let info = parse_qemu_img_info(json).unwrap();
        assert_eq!(info.format, "qcow2");
        assert_eq!(info.virtual_size, 21474836480);
    }

    #[test]
    fn rejects_missing_format() {
        let json = r#"{"virtual-size":1024}"#;
        let error = parse_qemu_img_info(json).unwrap_err();
        assert!(error.to_string().contains("format"), "error: {error}");
    }

    #[test]
    fn rejects_unknown_format() {
        let json = r#"{"format":"mystery","virtual-size":1024}"#;
        let error = parse_qemu_img_info(json).unwrap_err();
        assert!(
            error.to_string().contains("unknown image format"),
            "error: {error}"
        );
    }

    use crate::commands::emu::arch::GuestArch;
    use crate::commands::emu::discovery::Firmware;
    use crate::commands::emu::discovery::ImageCandidate;
    use tempfile::tempdir;

    fn candidate(
        dir: &std::path::Path,
        file: &str,
        arch: GuestArch,
        fw: Firmware,
    ) -> ImageCandidate {
        let path = dir.join(file);
        std::fs::write(&path, b"img").unwrap();
        ImageCandidate {
            id: "win11".to_string(),
            path,
            display_name: "Win11".to_string(),
            arch,
            arch_inferred: true,
            firmware: fw,
        }
    }

    #[test]
    fn writes_image_table_preserving_dir_and_comments() {
        let dir = tempdir().unwrap();
        let emu_toml = dir.path().join("emu.toml");
        std::fs::write(
            &emu_toml,
            "# my emus\ndir = \"~/vms\"\n\n[images.existing]\npath = \"/a/old.qcow2\"\n",
        )
        .unwrap();
        let cand = candidate(dir.path(), "win11.qcow2", GuestArch::X86_64, Firmware::Uefi);
        let id = write_image_entry(&emu_toml, &cand).unwrap();
        assert_eq!(id, "win11");
        let written = std::fs::read_to_string(&emu_toml).unwrap();
        assert!(written.contains("# my emus"), "comment dropped: {written}");
        assert!(
            written.contains("dir = \"~/vms\""),
            "dir dropped: {written}"
        );
        assert!(
            written.contains("[images.win11]"),
            "table missing: {written}"
        );
        assert!(
            written.contains("arch = \"x86_64\""),
            "arch missing: {written}"
        );
        assert!(
            written.contains("firmware = \"uefi\""),
            "firmware missing: {written}"
        );
    }

    #[test]
    fn skips_when_id_already_registered_by_canonical_path() {
        let dir = tempdir().unwrap();
        let emu_toml = dir.path().join("emu.toml");
        let cand = candidate(dir.path(), "win11.qcow2", GuestArch::X86_64, Firmware::Bios);
        let canonical = cand.path.canonicalize().unwrap();
        std::fs::write(
            &emu_toml,
            format!("[images.win11]\npath = \"{}\"\n", canonical.display()),
        )
        .unwrap();
        let before = std::fs::read_to_string(&emu_toml).unwrap();
        let id = write_image_entry(&emu_toml, &cand).unwrap();
        assert_eq!(id, "win11");
        let after = std::fs::read_to_string(&emu_toml).unwrap();
        assert_eq!(before, after, "must not append a duplicate entry");
    }

    #[test]
    fn fails_on_malformed_toml_without_appending() {
        let dir = tempdir().unwrap();
        let emu_toml = dir.path().join("emu.toml");
        std::fs::write(&emu_toml, "this is = = not valid toml\n").unwrap();
        let cand = candidate(dir.path(), "win11.qcow2", GuestArch::X86_64, Firmware::Bios);
        let error = write_image_entry(&emu_toml, &cand).unwrap_err();
        assert!(
            error.to_string().contains("emu.toml") || error.to_string().contains("parse"),
            "error: {error}"
        );
        let after = std::fs::read_to_string(&emu_toml).unwrap();
        assert!(
            !after.contains("[images.win11]"),
            "appended on malformed: {after}"
        );
    }

    #[test]
    fn relative_or_symlinked_image_path_still_dedups() {
        let dir = tempdir().unwrap();
        let emu_toml = dir.path().join("emu.toml");
        let cand = candidate(dir.path(), "win11.qcow2", GuestArch::X86_64, Firmware::Bios);
        let canonical = cand.path.canonicalize().unwrap();
        std::fs::write(
            &emu_toml,
            format!(
                "[images.other]\npath = \"{}/./win11.qcow2\"\n",
                canonical.parent().unwrap().display()
            ),
        )
        .unwrap();
        let id = write_image_entry(&emu_toml, &cand).unwrap();
        assert_eq!(id, "win11");
        let after = std::fs::read_to_string(&emu_toml).unwrap();
        assert!(
            !after.contains("[images.win11]"),
            "canonical-equal path should dedup, got: {after}"
        );
    }
}
