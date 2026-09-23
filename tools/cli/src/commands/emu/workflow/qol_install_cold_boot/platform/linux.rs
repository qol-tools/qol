use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use crate::commands::emu::BootedVm;
use crate::progress::{step_label, StepKind};

use super::desktop::{
    command, connect_after_reboot, connect_desktop_guest, install_payload, reboot_guest_cleanly,
    require_exec, wait_for_autostart_tray,
};
use super::Verdict;

const INSTALLER_PATH: &str = "/home/qol/.local/bin/qol-tray-install";
const TRAY_PATH: &str = "/home/qol/.local/bin/qol-tray";
const AUTOSTART_PATH: &str = "/home/qol/.config/autostart/qol-tray.desktop";
const COMMAND_TIMEOUT: Duration = Duration::from_secs(90);
const BOOT_SETTLE: Duration = Duration::from_secs(45);

pub(super) fn run(vm: &BootedVm) -> Result<Verdict> {
    let mut guest = connect_desktop_guest(vm)?;
    install_payload(&mut guest)?;
    require_exec(
        &mut guest,
        command(INSTALLER_PATH, &["--source", TRAY_PATH]),
        COMMAND_TIMEOUT,
    )?;
    let before = require_exec(
        &mut guest,
        command("/usr/bin/cat", &[AUTOSTART_PATH]),
        COMMAND_TIMEOUT,
    )?
    .stdout;
    let expected_exec = format!("Exec=\"{TRAY_PATH}\"");
    if !before.lines().any(|line| line == expected_exec) {
        bail!("installer did not write the expected autostart Exec line");
    }
    step_label(
        "install",
        StepKind::Success,
        "the production installer wrote a valid tray autostart entry",
    );

    let boot_started = Instant::now();
    reboot_guest_cleanly(&mut guest)?;
    drop(guest);
    thread::sleep(BOOT_SETTLE);

    let mut guest = connect_after_reboot(vm)?;
    wait_for_autostart_tray(&mut guest)?;
    let after = require_exec(
        &mut guest,
        command("/usr/bin/cat", &[AUTOSTART_PATH]),
        COMMAND_TIMEOUT,
    )?
    .stdout;
    if after != before {
        bail!("the autostart entry changed across reboot");
    }
    let pid = require_exec(
        &mut guest,
        command("/usr/bin/pgrep", &["-o", "-x", "qol-tray"]),
        COMMAND_TIMEOUT,
    )?
    .stdout
    .trim()
    .to_string();
    if pid.parse::<u32>().is_err() {
        bail!("autostarted tray did not have a valid process ID");
    }
    let proc_exe = format!("/proc/{pid}/exe");
    let binary = require_exec(
        &mut guest,
        command("/usr/bin/readlink", &[&proc_exe]),
        COMMAND_TIMEOUT,
    )?
    .stdout
    .trim()
    .to_string();
    if binary != TRAY_PATH {
        bail!("autostarted tray binary was {binary}, expected {TRAY_PATH}");
    }
    step_label(
        "cold-boot",
        StepKind::Success,
        "the installer-created entry launched the production tray after reboot",
    );
    Ok(Verdict {
        pass: true,
        traces: vec![
            expected_exec,
            format!("tray_exe={binary}"),
            format!(
                "reboot_to_verified_tray_ms={}",
                boot_started.elapsed().as_millis()
            ),
        ],
        artifacts: Vec::new(),
    })
}
