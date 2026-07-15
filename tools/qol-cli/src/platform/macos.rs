use super::PlatformOps;
use anyhow::Result;
use std::process::Command;

pub(crate) struct Platform;

impl PlatformOps for Platform {
    fn os_name(&self) -> &'static str {
        "macos"
    }

    fn exe_name(&self, name: &str) -> String {
        name.to_string()
    }

    fn stop_qol_tray(&self) -> Result<()> {
        let _ = Command::new("pkill").args(["-x", "qol-tray"]).status();
        Ok(())
    }

    fn qol_tray_running(&self) -> bool {
        Command::new("pgrep")
            .args(["-x", "qol-tray"])
            .output()
            .is_ok_and(|output| output.status.success())
    }

    fn copy_to_clipboard(&self, text: &str) -> Result<()> {
        super::pipe_to_clipboard("pbcopy", &[], text)
    }

    fn available_memory_mb(&self) -> Option<u64> {
        let output = Command::new("vm_stat").output().ok()?;
        if !output.status.success() {
            return None;
        }
        parse_available_memory_mb(&String::from_utf8_lossy(&output.stdout))
    }
}

fn parse_available_memory_mb(output: &str) -> Option<u64> {
    let page_size = output
        .lines()
        .next()?
        .split("page size of ")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    let mut pages = output.lines().skip(1).filter_map(|line| {
        let (name, value) = line.split_once(':')?;
        matches!(name, "Pages free" | "Pages inactive" | "Pages speculative")
            .then(|| value.trim().trim_end_matches('.').parse::<u64>().ok())?
    });
    let mut found = false;
    let available_pages = pages.try_fold(0_u64, |total, pages| {
        found = true;
        total.checked_add(pages)
    })?;
    if !found {
        return None;
    }
    available_pages
        .checked_mul(page_size)
        .map(|bytes| bytes / 1_048_576)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vm_stat_available_pages() {
        let output = "Mach Virtual Memory Statistics: (page size of 4096 bytes)\nPages free: 256.\nPages active: 999.\nPages inactive: 512.\nPages speculative: 256.\n";
        assert_eq!(parse_available_memory_mb(output), Some(4));
        assert_eq!(
            parse_available_memory_mb(
                "Mach Virtual Memory Statistics: (page size of 4096 bytes)\nPages active: 999.\n"
            ),
            None
        );
        assert_eq!(parse_available_memory_mb("bad"), None);
    }
}
