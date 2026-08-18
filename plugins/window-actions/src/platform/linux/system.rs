use std::fs;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use qol_windowing::{WindowId, WindowOps, WindowRect};

use crate::restore::WindowSystem;

const WINDOW_STATE_TIMEOUT: Duration = Duration::from_secs(2);
const WINDOW_STATE_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, Copy, Default)]
pub(crate) struct X11WindowSystem;

impl WindowOps for X11WindowSystem {
    fn enumerate_windows(&self) -> Result<Vec<WindowId>, String> {
        let output = Command::new("xprop")
            .args(["-root", "_NET_CLIENT_LIST_STACKING"])
            .output()
            .map_err(|error| format!("Failed to run xprop: {error}"))?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let raw = String::from_utf8_lossy(&output.stdout);
        let mut window_ids: Vec<WindowId> = raw
            .split_whitespace()
            .map(|token| token.trim_matches(|c| c == ',' || c == '#'))
            .filter_map(WindowId::parse)
            .collect();
        window_ids.reverse();
        Ok(window_ids)
    }

    fn window_geometry(&self, _window_id: &WindowId) -> Result<Option<WindowRect>, String> {
        Err("window-actions: window geometry lookup is not implemented on Linux".to_string())
    }

    fn move_resize(&self, _window_id: &WindowId, _rect: WindowRect) -> Result<(), String> {
        Err("window-actions: window move/resize is not implemented on Linux".to_string())
    }

    fn focus_window(&self, window_id: &WindowId) -> Result<bool, String> {
        wait_for_window_activation(self, window_id)
    }

    fn minimize_window(&self, window_id: &WindowId) -> Result<bool, String> {
        let status = Command::new("xdotool")
            .args(["windowminimize", window_id.as_str()])
            .status()
            .map_err(|error| format!("Failed to run xdotool: {error}"))?;
        if !status.success() {
            return Ok(false);
        }
        wait_for_window_state(self, window_id, true)
    }

    fn restore_window(&self, window_id: &WindowId) -> Result<bool, String> {
        wait_for_window_activation(self, window_id)
    }

    fn active_window_id(&self) -> Result<Option<WindowId>, String> {
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

        Ok(WindowId::parse(&window_id))
    }
}

impl WindowSystem for X11WindowSystem {
    fn is_excluded_window_type(&self, window_id: &WindowId) -> Result<bool, String> {
        let output = run_output("xprop", &["-id", window_id.as_str(), "_NET_WM_WINDOW_TYPE"])?;
        Ok(output.contains("_NET_WM_WINDOW_TYPE_DESKTOP")
            || output.contains("_NET_WM_WINDOW_TYPE_DOCK"))
    }

    fn is_hidden_window(&self, window_id: &WindowId) -> Result<bool, String> {
        let output = run_output("xprop", &["-id", window_id.as_str(), "_NET_WM_STATE"])?;
        Ok(output.contains("_NET_WM_STATE_HIDDEN"))
    }

    fn is_launcher_window(&self, window_id: &WindowId) -> bool {
        if launcher_pid_matches(self.window_pid(window_id).ok().flatten()) {
            return true;
        }

        launcher_window_metadata_matches(window_id.as_str())
    }

    fn window_pid(&self, window_id: &WindowId) -> Result<Option<u32>, String> {
        let output = run_output("xprop", &["-id", window_id.as_str(), "_NET_WM_PID"])?;
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

fn wait_for_window_state(
    system: &X11WindowSystem,
    window_id: &WindowId,
    hidden: bool,
) -> Result<bool, String> {
    let deadline = Instant::now() + WINDOW_STATE_TIMEOUT;
    loop {
        if system.is_hidden_window(window_id)? == hidden {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(WINDOW_STATE_POLL_INTERVAL);
    }
}

fn wait_for_window_activation(
    system: &X11WindowSystem,
    window_id: &WindowId,
) -> Result<bool, String> {
    let deadline = Instant::now() + WINDOW_STATE_TIMEOUT;
    loop {
        let status = Command::new("wmctrl")
            .args(["-ia", window_id.as_str()])
            .status()
            .map_err(|error| format!("Failed to run wmctrl: {error}"))?;
        if !status.success() {
            return Ok(false);
        }
        let active = system.active_window_id()?;
        if !system.is_hidden_window(window_id)? && active.as_ref() == Some(window_id) {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(WINDOW_STATE_POLL_INTERVAL);
    }
}

pub(crate) fn run_cinnamon_eval(script: &str) -> Result<String, String> {
    let result = qol_platform::cinnamon::Session::connect()?.eval(script);
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "WINACT_EVAL",
        "outcome={} detail={:?}",
        if result.is_ok() { "ok" } else { "err" },
        result.as_deref().unwrap_or_else(|error| error)
    );
    result
}

pub(crate) fn run_status(command: &str, args: &[&str]) -> Result<(), String> {
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
    qol_conventions::launcher::MATCH_MARKERS
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
