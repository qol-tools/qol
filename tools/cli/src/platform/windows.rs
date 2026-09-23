use super::{OpenPathOutcome, PlatformOps};
use anyhow::{Context, Result};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) struct Platform;

impl PlatformOps for Platform {
    fn exe_name(&self, name: &str) -> String {
        format!("{name}.exe")
    }

    fn stop_qol_tray(&self) -> Result<()> {
        let _ = Command::new("taskkill")
            .args(["/IM", "qol-tray.exe", "/F"])
            .status();
        Ok(())
    }

    fn qol_tray_running(&self) -> bool {
        let Ok(output) = Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq qol-tray.exe", "/NH"])
            .output()
        else {
            return false;
        };
        String::from_utf8_lossy(&output.stdout)
            .to_ascii_lowercase()
            .contains("qol-tray.exe")
    }

    fn copy_to_clipboard(&self, text: &str) -> Result<()> {
        super::pipe_to_clipboard("clip", &[], text)
    }

    fn available_memory_mb(&self) -> Option<u64> {
        available_memory_mb()
    }

    fn home_dir(&self) -> Option<PathBuf> {
        env::var_os("USERPROFILE")
            .or_else(|| env::var_os("HOME"))
            .map(PathBuf::from)
    }

    fn open_path(&self, path: &Path) -> Result<OpenPathOutcome> {
        qol_apps::desktop_integration::open_with_default_app(path)
            .with_context(|| format!("could not open {}", path.display()))?;
        Ok(OpenPathOutcome::new(true))
    }

    fn supports_immutable_payload_build(&self) -> bool {
        false
    }

    fn open_text_file(&self, path: &Path) -> bool {
        qol_apps::desktop_integration::open_with_default_app(path).is_ok()
    }
}

#[repr(C)]
struct MemoryStatusEx {
    length: u32,
    memory_load: u32,
    total_physical: u64,
    available_physical: u64,
    total_page_file: u64,
    available_page_file: u64,
    total_virtual: u64,
    available_virtual: u64,
    available_extended_virtual: u64,
}

#[link(name = "kernel32")]
extern "system" {
    fn GlobalMemoryStatusEx(status: *mut MemoryStatusEx) -> i32;
}

fn available_memory_mb() -> Option<u64> {
    let mut status = MemoryStatusEx {
        length: u32::try_from(std::mem::size_of::<MemoryStatusEx>()).ok()?,
        memory_load: 0,
        total_physical: 0,
        available_physical: 0,
        total_page_file: 0,
        available_page_file: 0,
        total_virtual: 0,
        available_virtual: 0,
        available_extended_virtual: 0,
    };
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return None;
    }
    Some(status.available_physical / 1_048_576)
}
