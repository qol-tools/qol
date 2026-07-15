use super::PlatformOps;
use anyhow::{anyhow, Result};
use std::fs;
use std::process::Command;

pub(crate) struct Platform;

impl PlatformOps for Platform {
    fn os_name(&self) -> &'static str {
        "linux"
    }

    fn exe_name(&self, name: &str) -> String {
        name.to_string()
    }

    fn stop_qol_tray(&self) -> Result<()> {
        let _ = Command::new("pkill").args(["-x", "qol-tray"]).status();
        Ok(())
    }

    fn qol_tray_running(&self) -> bool {
        let Ok(output) = Command::new("pgrep").args(["-x", "qol-tray"]).output() else {
            return false;
        };
        output
            .stdout
            .split(|byte| *byte == b'\n')
            .filter_map(|line| std::str::from_utf8(line).ok())
            .filter_map(|line| line.trim().parse::<u32>().ok())
            .any(|pid| qol_process::is_pid_alive(pid) && !qol_process::is_pid_zombie(pid))
    }

    fn copy_to_clipboard(&self, text: &str) -> Result<()> {
        super::pipe_to_clipboard("wl-copy", &[], text)
            .or_else(|_| super::pipe_to_clipboard("xclip", &["-selection", "clipboard"], text))
            .or_else(|_| super::pipe_to_clipboard("xsel", &["--clipboard", "--input"], text))
            .map_err(|_| anyhow!("no clipboard tool found (install wl-copy, xclip, or xsel)"))
    }

    fn available_memory_mb(&self) -> Option<u64> {
        fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|content| parse_available_memory_mb(&content))
    }
}

fn parse_available_memory_mb(content: &str) -> Option<u64> {
    content.lines().find_map(|line| {
        let value = line.strip_prefix("MemAvailable:")?.trim();
        let kib = value.strip_suffix("kB")?.trim().parse::<u64>().ok()?;
        Some(kib / 1024)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_available_memory_from_proc_meminfo() {
        let cases = [
            ("MemTotal: 100 kB\nMemAvailable: 2048 kB\n", Some(2)),
            ("MemAvailable: 1048576 kB\n", Some(1024)),
            ("MemAvailable: nope kB\n", None),
            ("MemAvailable: 100 bytes\n", None),
            ("MemTotal: 2048 kB\n", None),
            ("", None),
        ];
        for (content, expected) in cases {
            assert_eq!(parse_available_memory_mb(content), expected, "{content:?}");
        }
    }
}
