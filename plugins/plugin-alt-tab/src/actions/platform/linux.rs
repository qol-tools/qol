pub fn activate_window(window_id: u32) {
    eprintln!("[alt-tab/activate] xdotool windowactivate {}", window_id);
    match std::process::Command::new("xdotool")
        .arg("windowactivate")
        .arg("--sync")
        .arg(window_id.to_string())
        .status()
    {
        Ok(status) if status.success() => {
            eprintln!("[alt-tab/activate] xdotool succeeded for {}", window_id);
        }
        Ok(status) => {
            eprintln!(
                "[alt-tab/activate] xdotool failed for {} (exit={})",
                window_id,
                status.code().unwrap_or(-1)
            );
        }
        Err(e) => {
            eprintln!("[alt-tab/activate] xdotool spawn failed: {}", e);
        }
    }
}

pub fn close_window(window_id: u32) {
    std::process::Command::new("xdotool")
        .arg("windowclose")
        .arg(window_id.to_string())
        .status()
        .ok();
}

pub fn quit_app(window_id: u32) {
    // Get the PID of the window's owning process and send SIGTERM for a clean quit.
    let Ok(output) = std::process::Command::new("xdotool")
        .arg("getwindowpid")
        .arg(window_id.to_string())
        .output()
    else {
        return;
    };
    if let Ok(pid_str) = std::str::from_utf8(&output.stdout) {
        let pid = pid_str.trim();
        if !pid.is_empty() {
            std::process::Command::new("kill").arg(pid).status().ok();
        }
    }
}

pub fn minimize_window_by_id(window_id: u32) {
    std::process::Command::new("xdotool")
        .arg("windowminimize")
        .arg(window_id.to_string())
        .status()
        .ok();
}
