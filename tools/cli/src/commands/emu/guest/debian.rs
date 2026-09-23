use anyhow::{bail, Result};
use std::time::Duration;

use super::super::serial::SerialClient;
use super::GuestOs;

const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const PROMPT: &str = ":~#";
const ROOT_PASSWORD: &str = "qol";
const STICK_DEVICE: &str = "/dev/sda";
const STICK_WAIT_ATTEMPTS: u8 = 40;
const MAX_LOGIN_PROMPTS: usize = 20;

const STUB_LINES: [&str; 5] = [
    "#!/bin/sh",
    "echo qol-stub start",
    "mkdir -p /tmp/qol-stub",
    "date > /tmp/qol-stub/scratch",
    "echo qol-stub done",
];

pub(crate) struct DebianNocloud;

impl GuestOs for DebianNocloud {
    fn ensure_root_shell(&self, serial: &mut SerialClient) -> Result<()> {
        serial.send_line("")?;
        for _ in 0..MAX_LOGIN_PROMPTS {
            let (marker, _) = serial.wait_for_any(
                &[
                    PROMPT,
                    "login: ",
                    "root password",
                    "password again",
                    "Password:",
                    "(empty to skip",
                    "Press any key to proceed",
                    "Login incorrect",
                ],
                LOGIN_TIMEOUT,
            )?;
            match marker {
                0 => return Ok(()),
                1 => serial.send_line("root")?,
                2..=4 => serial.send_line(ROOT_PASSWORD)?,
                5 | 6 => serial.send_line("")?,
                7 => bail!("root login rejected on the serial console"),
                _ => unreachable!("wait_for_any returned an index beyond its markers"),
            }
        }
        bail!("no root shell after answering {MAX_LOGIN_PROMPTS} serial prompts")
    }

    fn launch_qol_from_stick(&self, serial: &mut SerialClient) -> Result<()> {
        serial.run_command(&wait_for_stick_command(), COMMAND_TIMEOUT)?;
        serial.run_command(&format!("mkfs.ext2 -q {STICK_DEVICE}"), COMMAND_TIMEOUT)?;
        serial.run_command(&format!("mount {STICK_DEVICE} /mnt"), COMMAND_TIMEOUT)?;
        let lines = STUB_LINES
            .iter()
            .map(|line| format!("'{line}'"))
            .collect::<Vec<_>>()
            .join(" ");
        serial.run_command(
            &format!("printf '%s\\n' {lines} > /mnt/qol-stub.sh"),
            COMMAND_TIMEOUT,
        )?;
        serial.run_command("sh /mnt/qol-stub.sh", COMMAND_TIMEOUT)?;
        serial.run_command("umount /mnt", COMMAND_TIMEOUT)?;
        Ok(())
    }

    fn reboot_and_relogin(&self, serial: &mut SerialClient) -> Result<()> {
        serial.send_line("reboot")?;
        serial.wait_for("reboot: Restarting system", LOGIN_TIMEOUT)?;
        self.ensure_root_shell(serial)
    }

    fn list_qol_traces(&self, serial: &mut SerialClient) -> Result<Vec<String>> {
        let output = serial.run_command(
            "find / -xdev -iname '*qol*' 2>/dev/null || true",
            COMMAND_TIMEOUT,
        )?;
        Ok(parse_traces(&output))
    }
}

fn wait_for_stick_command() -> String {
    format!(
        "i=0; while [ $i -lt {STICK_WAIT_ATTEMPTS} ]; do [ -b {STICK_DEVICE} ] && break; i=$((i+1)); sleep 0.25; done; [ -b {STICK_DEVICE} ] || (lsblk; ls -l /dev/sd* /dev/vd* 2>/dev/null; false)"
    )
}

fn parse_traces(output: &str) -> Vec<String> {
    output
        .lines()
        .map(clean_console_line)
        .map(|line| line.trim().to_string())
        .filter(|line| line.starts_with('/'))
        .collect()
}

fn clean_console_line(raw: &str) -> String {
    let mut cleaned = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for code in chars.by_ref() {
                if ('@'..='~').contains(&code) {
                    break;
                }
            }
            continue;
        }
        if ch == '\r' {
            cleaned.clear();
            continue;
        }
        cleaned.push(ch);
    }
    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_for_stick_command_checks_expected_device() {
        let command = wait_for_stick_command();
        assert!(command.contains("[ -b /dev/sda ]"), "command: {command}");
        assert!(command.contains("lsblk"), "command: {command}");
    }

    #[test]
    fn parse_traces_keeps_only_absolute_paths() {
        let output = "find / -xdev -iname '*qol*' 2>/dev/null || true; echo QOL-\"RC\"-$?\r\n\x1b[?2004l\r/root/.qol-residue\r\n/etc/qol.conf\r\nQOL-RC-";
        assert_eq!(
            parse_traces(output),
            vec!["/root/.qol-residue", "/etc/qol.conf"]
        );
    }

    #[test]
    fn parse_traces_returns_empty_for_clean_output() {
        let output =
            "find / -xdev -iname '*qol*' 2>/dev/null || true; echo QOL-\"RC\"-$?\r\nQOL-RC-";
        assert_eq!(parse_traces(output), Vec::<String>::new());
    }
}
