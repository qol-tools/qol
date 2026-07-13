use anyhow::{bail, Context, Result};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use super::media;

pub(crate) fn free_qmp_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("failed to probe a free qmp port")?;
    Ok(listener
        .local_addr()
        .context("failed to read qmp probe address")?
        .port())
}

pub(crate) fn spawn_qemu(qemu_system: &Path, args: &[String]) -> Result<Child> {
    Command::new(qemu_system)
        .args(args)
        .stdin(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to spawn {}", qemu_system.display()))
}

pub(crate) fn ensure_usb_stick(run_dir: &Path, qemu_img: &Path) -> Result<PathBuf> {
    let stick = run_dir.join("usb-stick.raw");
    if stick.is_file() {
        return Ok(stick);
    }
    let status = Command::new(qemu_img)
        .arg("create")
        .arg("-f")
        .arg("raw")
        .arg(&stick)
        .arg("16M")
        .status()
        .with_context(|| format!("failed to run {}", qemu_img.display()))?;
    if !status.success() {
        bail!("qemu-img create failed for {}", stick.display());
    }
    Ok(stick)
}

pub(crate) fn ensure_payload_stick(
    run_dir: &Path,
    qemu_img: &Path,
    payload_root: &Path,
) -> Result<PathBuf> {
    let stick = run_dir.join("usb-stick.raw");
    let pending = run_dir.join("usb-stick.raw.pending");
    remove_if_present(&pending)?;
    create_raw_stick(qemu_img, &pending, "128M")?;
    let mke2fs = super::find_on_path("mke2fs")
        .or_else(|| super::find_on_path("mkfs.ext2"))
        .context("mke2fs is required to build the verified-uninstall payload")?;
    let status = Command::new(&mke2fs)
        .args(payload_mkfs_args(payload_root, &pending))
        .status()
        .with_context(|| format!("failed to run {}", mke2fs.display()))?;
    if !status.success() {
        let _ = std::fs::remove_file(&pending);
        bail!("mke2fs failed for {} with {status}", pending.display())
    }
    remove_if_present(&stick)?;
    std::fs::rename(&pending, &stick).with_context(|| {
        format!(
            "failed to publish payload stick {} as {}",
            pending.display(),
            stick.display()
        )
    })?;
    Ok(stick)
}

fn remove_if_present(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to remove {}", path.display())),
    }
}

fn create_raw_stick(qemu_img: &Path, stick: &Path, size: &str) -> Result<()> {
    let status = Command::new(qemu_img)
        .args(["create", "-f", "raw"])
        .arg(stick)
        .arg(size)
        .status()
        .with_context(|| format!("failed to run {}", qemu_img.display()))?;
    if status.success() {
        return Ok(());
    }
    bail!("qemu-img create failed for {}", stick.display())
}

fn payload_mkfs_args(payload_root: &Path, stick: &Path) -> Vec<std::ffi::OsString> {
    ["-q", "-F", "-t", "ext2", "-d"]
        .into_iter()
        .map(std::ffi::OsString::from)
        .chain([
            payload_root.as_os_str().to_owned(),
            stick.as_os_str().to_owned(),
        ])
        .collect()
}

pub(crate) fn teardown(run_dir: &Path) -> Result<Vec<PathBuf>> {
    media::cleanup_artifacts(run_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn free_qmp_port_returns_bindable_port() {
        let port = free_qmp_port().unwrap();
        assert_ne!(port, 0);
    }

    #[test]
    fn ensure_usb_stick_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("qol-emu-stick-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("usb-stick.raw"), b"existing").unwrap();
        let stick = ensure_usb_stick(&dir, Path::new("/nonexistent/qemu-img")).unwrap();
        assert_eq!(stick, dir.join("usb-stick.raw"));
        assert_eq!(fs::read(&stick).unwrap(), b"existing");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn payload_mkfs_args_populate_the_stick_from_the_staging_directory() {
        assert_eq!(
            payload_mkfs_args(Path::new("/payload"), Path::new("/run/stick.raw")),
            ["-q", "-F", "-t", "ext2", "-d", "/payload", "/run/stick.raw"]
                .map(std::ffi::OsString::from)
        );
    }

    #[test]
    fn teardown_removes_disk_images_and_keeps_evidence() {
        let dir = std::env::temp_dir().join(format!("qol-emu-teardown-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let files = [
            "overlay.qcow2",
            "overlay-snap-1.qcow2",
            "usb-stick.raw",
            "usb-stick.raw.pending",
            "manual.qcow2",
            "report.json",
            "qemu-command.txt",
            "screenshot-1.ppm",
        ];
        for name in files {
            fs::write(dir.join(name), b"x").unwrap();
        }
        let removed = teardown(&dir).unwrap();
        let mut expected_removed = vec![
            dir.join("overlay-snap-1.qcow2"),
            dir.join("overlay.qcow2"),
            dir.join("usb-stick.raw"),
            dir.join("usb-stick.raw.pending"),
        ];
        expected_removed.sort();
        assert_eq!(removed, expected_removed);
        let expectations = [
            ("overlay.qcow2", false),
            ("overlay-snap-1.qcow2", false),
            ("usb-stick.raw", false),
            ("usb-stick.raw.pending", false),
            ("manual.qcow2", true),
            ("report.json", true),
            ("qemu-command.txt", true),
            ("screenshot-1.ppm", true),
        ];
        for (name, should_exist) in expectations {
            assert_eq!(dir.join(name).exists(), should_exist, "file: {name}");
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
