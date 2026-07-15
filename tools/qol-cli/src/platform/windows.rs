use super::PlatformOps;
use anyhow::Result;
use std::process::Command;

pub(crate) struct Platform;

impl PlatformOps for Platform {
    fn os_name(&self) -> &'static str {
        "windows"
    }

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
