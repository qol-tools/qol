use std::fs;
use std::process::Command;

use crate::restore::WindowSystem;

const LAUNCHER_MATCH_MARKERS: [&str; 4] = [
    "qol-tray-launcher",
    "plugin-launcher",
    "qol-launcher",
    "qol launcher",
];

#[derive(Clone, Copy, Default)]
pub struct X11WindowSystem;

impl WindowSystem for X11WindowSystem {
    fn active_window_id(&self) -> Result<Option<String>, String> {
        let output = Command::new("xdotool")
            .arg("getactivewindow")
            .output()
            .map_err(|error| format!("Failed to run xdotool: {error}"))?;

        if !output.status.success() {
            return Ok(None);
        }

        let window_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if window_id.is_empty() {
            return Ok(None);
        }

        Ok(Some(window_id))
    }

    fn minimize_window(&self, window_id: &str) -> Result<bool, String> {
        let status = Command::new("xdotool")
            .args(["windowminimize", window_id])
            .status()
            .map_err(|error| format!("Failed to run xdotool: {error}"))?;
        Ok(status.success())
    }

    fn window_rect(&self, _window_id: &str) -> Option<[f64; 4]> {
        None
    }

    fn stacking_window_ids(&self) -> Result<Vec<String>, String> {
        let output = Command::new("xprop")
            .args(["-root", "_NET_CLIENT_LIST_STACKING"])
            .output()
            .map_err(|error| format!("Failed to run xprop: {error}"))?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let raw = String::from_utf8_lossy(&output.stdout);
        let mut window_ids: Vec<String> = raw
            .split_whitespace()
            .map(|token| token.trim_matches(|c| c == ',' || c == '#').to_string())
            .filter(|id| self.is_window_id(id))
            .collect();
        window_ids.reverse();
        Ok(window_ids)
    }

    fn is_window_id(&self, id: &str) -> bool {
        is_window_id(id)
    }

    fn normalize_window_id(&self, window_id: &str) -> Option<String> {
        normalize_window_id(window_id)
    }

    fn is_excluded_window_type(&self, window_id: &str) -> Result<bool, String> {
        let output = run_output("xprop", &["-id", window_id, "_NET_WM_WINDOW_TYPE"])?;
        Ok(
            output.contains("_NET_WM_WINDOW_TYPE_DESKTOP")
                || output.contains("_NET_WM_WINDOW_TYPE_DOCK"),
        )
    }

    fn is_hidden_window(&self, window_id: &str) -> Result<bool, String> {
        let output = run_output("xprop", &["-id", window_id, "_NET_WM_STATE"])?;
        Ok(output.contains("_NET_WM_STATE_HIDDEN"))
    }

    fn is_launcher_window(&self, window_id: &str) -> bool {
        if launcher_pid_matches(self.window_pid(window_id).ok().flatten()) {
            return true;
        }

        launcher_window_metadata_matches(window_id)
    }

    fn activate_window(&self, window_id: &str) -> Result<bool, String> {
        let status = Command::new("wmctrl")
            .args(["-ia", window_id])
            .status()
            .map_err(|error| format!("Failed to run wmctrl: {error}"))?;
        Ok(status.success())
    }

    fn restore_rect(&self, _window_id: &str, _rect: [f64; 4]) -> Result<(), String> {
        Ok(())
    }

    fn window_pid(&self, window_id: &str) -> Result<Option<u32>, String> {
        let output = run_output("xprop", &["-id", window_id, "_NET_WM_PID"])?;
        let pid = output
            .split('=')
            .nth(1)
            .map(str::trim)
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse::<u32>().ok());
        Ok(pid)
    }

    fn process_start_ticks(&self, pid: u32) -> Option<u64> {
        process_start_ticks(pid)
    }
}

pub fn run_cinnamon_eval(script: &str) -> Result<String, String> {
    let output = Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "org.Cinnamon",
            "--object-path",
            "/org/Cinnamon",
            "--method",
            "org.Cinnamon.Eval",
            script,
        ])
        .output()
        .map_err(|error| format!("Failed to run gdbus: {error}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(format!(
            "gdbus failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

pub fn run_status(command: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(command)
        .args(args)
        .status()
        .map_err(|error| format!("Failed to run {command}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{command} exited with status {status}"))
    }
}

fn run_output(command: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(command)
        .args(args)
        .output()
        .map_err(|error| format!("Failed to run {command}: {error}"))?;

    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        Err(format!("{command} exited with status {}", output.status))
    } else {
        Err(format!("{command} failed: {stderr}"))
    }
}

fn run_output_optional(command: &str, args: &[&str]) -> String {
    run_output(command, args).unwrap_or_default()
}

fn is_window_id(id: &str) -> bool {
    id.starts_with("0x") && id.len() > 2 && id.chars().skip(2).all(|c| c.is_ascii_hexdigit())
}

fn normalize_window_id(window_id: &str) -> Option<String> {
    if is_window_id(window_id) {
        return Some(window_id.to_ascii_lowercase());
    }

    let numeric = window_id.trim().parse::<u64>().ok()?;
    Some(format!("0x{numeric:x}"))
}

fn launcher_pid_matches(pid: Option<u32>) -> bool {
    let Some(pid) = pid else {
        return false;
    };

    launcher_process_cmdline_matches(pid) || launcher_process_name_matches(pid)
}

fn launcher_process_cmdline_matches(pid: u32) -> bool {
    let Some(cmdline) = process_cmdline(pid) else {
        return false;
    };

    launcher_text_matches(&cmdline)
}

fn launcher_process_name_matches(pid: u32) -> bool {
    let Some(name) = process_name(pid) else {
        return false;
    };

    launcher_text_matches(&name)
}

fn launcher_window_metadata_matches(window_id: &str) -> bool {
    let class = run_output_optional("xprop", &["-id", window_id, "WM_CLASS"]);
    let name = run_output_optional("xprop", &["-id", window_id, "_NET_WM_NAME"]);
    let app_id = run_output_optional("xprop", &["-id", window_id, "_GTK_APPLICATION_ID"]);
    let haystack = format!("{class}\n{name}\n{app_id}");
    launcher_text_matches(&haystack)
}

fn launcher_text_matches(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    LAUNCHER_MATCH_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

fn process_name(pid: u32) -> Option<String> {
    let path = format!("/proc/{pid}/comm");
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
}

fn process_cmdline(pid: u32) -> Option<String> {
    let path = format!("/proc/{pid}/cmdline");
    let data = fs::read(path).ok()?;
    if data.is_empty() {
        return None;
    }

    Some(
        data.split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf8_lossy(part).into_owned())
            .collect::<Vec<String>>()
            .join(" "),
    )
}

fn process_start_ticks(pid: u32) -> Option<u64> {
    let path = format!("/proc/{pid}/stat");
    let stat = fs::read_to_string(path).ok()?;
    let (_, rest) = stat.split_once(") ")?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    fields.get(19)?.parse::<u64>().ok()
}
