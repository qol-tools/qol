use anyhow::{bail, Context, Result};
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const DISPOSABLE_FILES: [&str; 1] = ["overlay.qcow2"];

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

pub(crate) fn teardown(run_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    for name in DISPOSABLE_FILES {
        let path = run_dir.join(name);
        if path.is_file() {
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove {}", path.display()))?;
            removed.push(path);
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn teardown_removes_overlay_and_keeps_artifacts() {
        let dir = std::env::temp_dir().join(format!("qol-emu-teardown-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("overlay.qcow2"), b"x").unwrap();
        fs::write(dir.join("report.json"), b"{}").unwrap();
        fs::write(dir.join("qemu-command.txt"), b"qemu").unwrap();
        let removed = teardown(&dir).unwrap();
        assert_eq!(removed, vec![dir.join("overlay.qcow2")]);
        let expectations = [
            ("overlay.qcow2", false),
            ("report.json", true),
            ("qemu-command.txt", true),
        ];
        for (name, should_exist) in expectations {
            assert_eq!(dir.join(name).exists(), should_exist, "file: {name}");
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
