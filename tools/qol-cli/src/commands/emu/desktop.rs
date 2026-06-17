use anyhow::{bail, Context, Result};
use serde_json::json;
use std::ffi::OsString;
use std::time::Duration;

use crate::progress::{print_hint, print_title, step_label, StepKind};

use super::guest::{DebianNocloud, GuestOs};
use super::{boot_vm, finalize_vm, serial, shutdown_vm};

const SYNTAX: &str = "qol emu desktop mintish <environment>";
const SERIAL_TIMEOUT: Duration = Duration::from_secs(10);
const APT_TIMEOUT: Duration = Duration::from_secs(1800);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const DESKTOP_PACKAGES: &[&str] = &[
    "task-cinnamon-desktop",
    "lightdm",
    "slick-greeter",
    "sudo",
    "dbus-x11",
    "mint-y-icons",
    "papirus-icon-theme",
    "arc-theme",
    "firefox-esr",
    "xterm",
    "qemu-guest-agent",
];

pub(crate) fn cmd_desktop(args: &[OsString], verbose: bool) -> Result<()> {
    if args.len() != 2 || args[0].to_str() != Some("mintish") {
        bail!("usage: {SYNTAX}");
    }
    let target = args[1]
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("environment id is not valid UTF-8"))?;
    print_title("qol emu desktop");
    print_hint(verbose);
    let mut vm = boot_vm(target, "desktop", verbose)?;
    let setup = prepare_mintish(vm.serial_port);
    if let Err(error) = setup {
        let detail = error.to_string();
        let exit = shutdown_vm(&mut vm).context("desktop setup failed and qemu did not stop")?;
        let report = desktop_report("error", Some(&detail));
        let (report_path, removed) = finalize_vm(vm, exit, Some(report), "desktop")?;
        step_label(
            "clean",
            StepKind::Success,
            &format!("removed {} disposable file(s)", removed.len()),
        );
        step_label("report", StepKind::Info, &report_path.display().to_string());
        bail!("{detail}");
    }
    step_label(
        "desktop",
        StepKind::Success,
        "mint-ish Cinnamon session is running",
    );
    step_label(
        "running",
        StepKind::Success,
        "close the VM window to end the run",
    );
    let exit = vm.child.wait().context("failed to wait for qemu")?;
    let (report_path, removed) =
        finalize_vm(vm, exit, Some(desktop_report("pass", None)), "desktop")?;
    step_label(
        "clean",
        StepKind::Success,
        &format!("removed {} disposable file(s)", removed.len()),
    );
    step_label("report", StepKind::Info, &report_path.display().to_string());
    if !exit.success() {
        bail!("qemu exited with {exit}");
    }
    Ok(())
}

fn prepare_mintish(serial_port: u16) -> Result<()> {
    let mut serial = serial::connect(serial_port, SERIAL_TIMEOUT)?;
    let os = DebianNocloud;
    step_label("login", StepKind::Pending, "waiting for a root shell");
    os.ensure_root_shell(&mut serial)?;
    step_label("login", StepKind::Success, "root shell over serial");
    for step in setup_steps() {
        step_label(step.label, StepKind::Pending, step.detail);
        serial.run_command(&step.command, step.timeout)?;
        step_label(step.label, StepKind::Success, step.done);
    }
    Ok(())
}

fn desktop_report(verdict: &str, error: Option<&str>) -> serde_json::Value {
    let mut report = json!({
        "id": "mintish-desktop",
        "verdict": verdict,
    });
    if let Some(error) = error {
        report["error"] = json!(error);
    }
    report
}

struct SetupStep {
    label: &'static str,
    detail: &'static str,
    done: &'static str,
    command: String,
    timeout: Duration,
}

fn setup_steps() -> Vec<SetupStep> {
    vec![
        SetupStep {
            label: "apt",
            detail: "updating package indexes",
            done: "package indexes updated",
            command: "export DEBIAN_FRONTEND=noninteractive; apt-get update".to_string(),
            timeout: APT_TIMEOUT,
        },
        SetupStep {
            label: "desktop",
            detail: "installing Cinnamon and Mint-ish packages",
            done: "desktop packages installed",
            command: install_command(),
            timeout: APT_TIMEOUT,
        },
        SetupStep {
            label: "user",
            detail: "creating autologin desktop user",
            done: "desktop user ready",
            command: "id -u mint >/dev/null 2>&1 || useradd -m -s /bin/bash mint; echo 'mint:qol' | chpasswd; usermod -aG sudo mint".to_string(),
            timeout: COMMAND_TIMEOUT,
        },
        SetupStep {
            label: "login",
            detail: "configuring LightDM autologin",
            done: "autologin configured",
            command: lightdm_command(),
            timeout: COMMAND_TIMEOUT,
        },
        SetupStep {
            label: "theme",
            detail: "applying Mint-ish Cinnamon defaults",
            done: "desktop defaults applied",
            command: theme_command(),
            timeout: COMMAND_TIMEOUT,
        },
        SetupStep {
            label: "start",
            detail: "starting graphical session",
            done: "graphical session started",
            command: "systemctl set-default graphical.target; systemctl enable lightdm; systemctl restart lightdm || systemctl start lightdm".to_string(),
            timeout: COMMAND_TIMEOUT,
        },
    ]
}

fn install_command() -> String {
    format!(
        "export DEBIAN_FRONTEND=noninteractive; echo 'lightdm shared/default-x-display-manager select lightdm' | debconf-set-selections; apt-get install -y {}",
        DESKTOP_PACKAGES.join(" ")
    )
}

fn lightdm_command() -> String {
    format!(
        "mkdir -p /etc/lightdm/lightdm.conf.d; {}",
        printf_lines(
            "/etc/lightdm/lightdm.conf.d/90-qol-mintish.conf",
            &[
                "[Seat:*]",
                "autologin-user=mint",
                "autologin-user-timeout=0",
                "user-session=cinnamon",
                "greeter-session=slick-greeter",
            ],
        )
    )
}

fn theme_command() -> String {
    [
        "mkdir -p /home/mint/.config",
        "chown -R mint:mint /home/mint/.config",
        "sudo -u mint dbus-run-session gsettings set org.cinnamon.desktop.interface icon-theme Mint-Y || true",
        "sudo -u mint dbus-run-session gsettings set org.cinnamon.desktop.interface gtk-theme Arc-Dark || true",
        "sudo -u mint dbus-run-session gsettings set org.cinnamon.desktop.wm.preferences theme Arc-Dark || true",
    ]
    .join("; ")
}

fn printf_lines(path: &str, lines: &[&str]) -> String {
    let args = lines
        .iter()
        .map(|line| format!("'{line}'"))
        .collect::<Vec<_>>()
        .join(" ");
    format!("printf '%s\\n' {args} > {path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_command_includes_cinnamon_lightdm_and_mintish_assets() {
        let command = install_command();
        for package in [
            "task-cinnamon-desktop",
            "lightdm",
            "slick-greeter",
            "mint-y-icons",
            "arc-theme",
        ] {
            assert!(command.contains(package), "package {package}: {command}");
        }
        assert!(
            command.contains("DEBIAN_FRONTEND=noninteractive"),
            "command: {command}"
        );
    }

    #[test]
    fn lightdm_command_writes_cinnamon_autologin_config() {
        let command = lightdm_command();
        for fragment in [
            "/etc/lightdm/lightdm.conf.d/90-qol-mintish.conf",
            "autologin-user=mint",
            "user-session=cinnamon",
            "greeter-session=slick-greeter",
        ] {
            assert!(command.contains(fragment), "fragment {fragment}: {command}");
        }
    }

    #[test]
    fn theme_command_applies_mint_y_icons_and_arc_window_theme() {
        let command = theme_command();
        assert!(command.contains("icon-theme Mint-Y"), "command: {command}");
        assert!(command.contains("gtk-theme Arc-Dark"), "command: {command}");
        assert!(
            command.contains("wm.preferences theme Arc-Dark"),
            "command: {command}"
        );
    }
}
