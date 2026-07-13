use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::time::Duration;

use super::super::serial::SerialClient;
use super::GuestOs;

const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(180);
const PROMPT: &str = ":~#";
const ROOT_PASSWORD: &str = "qol";
const STICK_DEVICE: &str = "/dev/sda";
const STICK_WAIT_ATTEMPTS: u8 = 40;
const MAX_LOGIN_PROMPTS: usize = 20;
const UNINSTALL_REPORT: &str = "/tmp/uninstall-report.json";

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

    fn verify_uninstall_from_stick(&self, serial: &mut SerialClient) -> Result<Value> {
        serial.run_command(&wait_for_stick_command(), COMMAND_TIMEOUT)?;
        serial.run_command(&format!("mount {STICK_DEVICE} /mnt"), COMMAND_TIMEOUT)?;
        serial.run_command(
            "chmod +x /mnt/qol-tray-install && HOME=/root /mnt/qol-tray-install --source /mnt/qol-tray-source",
            COMMAND_TIMEOUT,
        )?;
        serial.run_command(&installed_artifacts_command(), COMMAND_TIMEOUT)?;
        serial.run_command(
            &format!(
                "HOME=/root /mnt/qol-tray-install --uninstall --purge-data --json > {UNINSTALL_REPORT}"
            ),
            COMMAND_TIMEOUT,
        )?;
        let output = serial.run_command(&format!("cat {UNINSTALL_REPORT}"), COMMAND_TIMEOUT)?;
        let report = parse_uninstall_report(&output)?;
        validate_uninstall_report(&report)?;
        serial.run_command(&removed_artifacts_command(), COMMAND_TIMEOUT)?;
        serial.run_command(&format!("rm -f {UNINSTALL_REPORT}"), COMMAND_TIMEOUT)?;
        serial.run_command("umount /mnt", COMMAND_TIMEOUT)?;
        Ok(report)
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

fn installed_artifacts_command() -> String {
    [
        "/root/.local/bin/qol-tray",
        "/root/.local/bin/qol-tray.install-id",
        "/root/.config/autostart/qol-tray.desktop",
        "/root/.local/share/applications/qol-tray.desktop",
        "/root/.local/share/icons/hicolor/64x64/apps/qol-tray.png",
        "/root/.config/qol-tray/mode.json",
        "/root/.local/share/qol-tray/active-install-id",
    ]
    .into_iter()
    .map(|path| format!("test -e {path}"))
    .collect::<Vec<_>>()
    .join(" && ")
}

fn removed_artifacts_command() -> String {
    let paths = [
        "/root/.local/bin/qol-tray",
        "/root/.local/bin/qol-tray.install-id",
        "/root/.config/autostart/qol-tray.desktop",
        "/root/.local/share/applications/qol-tray.desktop",
        "/root/.local/share/icons/hicolor/64x64/apps/qol-tray.png",
        "/root/.config/qol-tray",
        "/root/.local/share/qol-tray",
        "/tmp/qol-tray",
    ];
    let mut checks = paths
        .into_iter()
        .map(|path| format!("test ! -e {path}"))
        .collect::<Vec<_>>();
    checks
        .push("! grep -qF '# >>> qol-tools shell hook >>>' /root/.bashrc 2>/dev/null".to_string());
    checks.join(" && ")
}

fn parse_uninstall_report(output: &str) -> Result<Value> {
    let start = output
        .find('{')
        .ok_or_else(|| anyhow!("uninstall report did not contain JSON"))?;
    let end = output
        .rfind('}')
        .ok_or_else(|| anyhow!("uninstall report JSON was incomplete"))?;
    serde_json::from_str(&output[start..=end]).context("uninstall report was not valid JSON")
}

fn validate_uninstall_report(report: &Value) -> Result<()> {
    if report.get("schema_version").and_then(Value::as_u64) != Some(1) {
        bail!("uninstall report has an unsupported schema version")
    }
    if report.get("status").and_then(Value::as_str) != Some("complete") {
        bail!("uninstall report was not complete: {report}")
    }
    if report.get("dry_run").and_then(Value::as_bool) != Some(false) {
        bail!("uninstall report did not confirm execution")
    }
    if report.get("purge_data").and_then(Value::as_bool) != Some(true) {
        bail!("uninstall report did not confirm data purge")
    }
    let actions = report
        .get("actions")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("uninstall report did not contain actions"))?;
    let incomplete = ["failed", "skipped_unowned", "skipped_dependency"];
    if actions.iter().any(|action| {
        action
            .get("result")
            .and_then(Value::as_str)
            .is_some_and(|result| incomplete.contains(&result))
    }) {
        bail!("uninstall report contains an incomplete action: {report}")
    }
    for (id, expected_result) in [
        ("shell_hook_bash", "updated"),
        ("autostart", "removed"),
        ("desktop_entry", "removed"),
        ("icon64", "removed"),
        ("mode_config", "removed"),
        ("active_install_id", "removed"),
        ("binary", "removed"),
        ("install_marker", "removed"),
        ("config_directory", "removed"),
        ("data_directory", "removed"),
    ] {
        let actual = actions
            .iter()
            .find(|action| action.get("id").and_then(Value::as_str) == Some(id))
            .and_then(|action| action.get("result"))
            .and_then(Value::as_str);
        if actual != Some(expected_result) {
            bail!("uninstall report did not prove {id} was {expected_result}; result={actual:?}")
        }
    }
    Ok(())
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
    fn artifact_commands_cover_install_and_purge_contracts() {
        let installed = installed_artifacts_command();
        let removed = removed_artifacts_command();
        for path in [
            "/root/.local/bin/qol-tray",
            "/root/.config/autostart/qol-tray.desktop",
        ] {
            assert!(installed.contains(path), "installed path={path}");
            assert!(removed.contains(path), "removed path={path}");
        }
        for path in [
            "/root/.config/qol-tray/mode.json",
            "/root/.local/share/qol-tray/active-install-id",
        ] {
            assert!(installed.contains(path), "installed path={path}");
        }
        for path in ["/root/.config/qol-tray", "/root/.local/share/qol-tray"] {
            assert!(removed.contains(path), "removed path={path}");
        }
        assert!(removed.contains("qol-tools shell hook"));
    }

    #[test]
    fn uninstall_report_parser_extracts_console_wrapped_json() {
        let report = complete_uninstall_report();
        let output = format!(
            "cat /tmp/uninstall-report.json\r\n{}\r\nQOL-RC-0",
            serde_json::to_string_pretty(&report).unwrap()
        );
        let report = parse_uninstall_report(&output).unwrap();
        validate_uninstall_report(&report).unwrap();
        assert_eq!(report["status"], "complete");
    }

    #[test]
    fn uninstall_report_validation_rejects_partial_actions() {
        let mut report = complete_uninstall_report();
        report["actions"][0]["result"] = Value::String("skipped_unowned".to_string());
        assert!(validate_uninstall_report(&report).is_err());
    }

    #[test]
    fn uninstall_report_validation_rejects_missing_required_evidence() {
        let mut report = complete_uninstall_report();
        report["actions"].as_array_mut().unwrap().pop();
        assert!(validate_uninstall_report(&report).is_err());
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

    fn complete_uninstall_report() -> Value {
        serde_json::json!({
            "schema_version": 1,
            "status": "complete",
            "dry_run": false,
            "purge_data": true,
            "actions": [
                {"id": "shell_hook_bash", "result": "updated"},
                {"id": "autostart", "result": "removed"},
                {"id": "desktop_entry", "result": "removed"},
                {"id": "icon64", "result": "removed"},
                {"id": "mode_config", "result": "removed"},
                {"id": "active_install_id", "result": "removed"},
                {"id": "binary", "result": "removed"},
                {"id": "install_marker", "result": "removed"},
                {"id": "config_directory", "result": "removed"},
                {"id": "data_directory", "result": "removed"}
            ]
        })
    }
}
