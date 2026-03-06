use std::path::Path;

pub(super) fn binary_name() -> &'static str {
    "qol-tray"
}

pub(super) fn spawn_delayed(binary: &Path) -> Result<(), String> {
    if let Some(command) = restart_command_override() {
        let script = format!("sleep 0.35; {}", command);
        if let Ok(()) = spawn_detached_script(&script, None) {
            return Ok(());
        }
    }
    spawn_detached_script("sleep 0.35; exec \"$1\"", Some(binary))
}

fn restart_command_override() -> Option<String> {
    let value = std::env::var("QOL_TRAY_RESTART_COMMAND").ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

fn configure_sh_command(cmd: &mut std::process::Command, script: &str, binary: Option<&Path>) {
    cmd.arg("sh").arg("-c").arg(script).arg("qol-tray-restart")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(path) = binary {
        cmd.arg(path);
    }
}

fn spawn_detached_script(script: &str, binary: Option<&Path>) -> Result<(), String> {
    let mut setsid = std::process::Command::new("setsid");
    setsid.arg("-f");
    configure_sh_command(&mut setsid, script, binary);
    match setsid.spawn() {
        Ok(_) => return Ok(()),
        Err(error) => log::warn!("setsid restart handoff failed: {}", error),
    }
    let mut nohup = std::process::Command::new("nohup");
    configure_sh_command(&mut nohup, script, binary);
    nohup.spawn().map(|_| ()).map_err(|e| e.to_string())
}
