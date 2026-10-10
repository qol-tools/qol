use anyhow::{Context, Result};
use qol_platform::native::registry::{read_binary, Hive};
use std::path::{Path, PathBuf};

use super::registry;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const STARTUP_APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const VALUE_NAME: &str = "qol-tray";
const LEGACY_STARTUP_FILE: &str = "qol-tray.cmd";

pub(in crate::installer) fn location() -> PathBuf {
    PathBuf::from(format!(r"HKCU\{RUN_KEY}\{VALUE_NAME}"))
}

pub(in crate::installer) fn read() -> Result<Option<PathBuf>> {
    if let Some(command) = registry::text(RUN_KEY, VALUE_NAME)? {
        if disabled_in_startup_apps()? {
            return Ok(None);
        }
        return Ok(parse_command(&command));
    }
    let Some(legacy) = legacy_startup_file() else {
        return Ok(None);
    };
    read_legacy_cmd(&legacy)
}

pub(in crate::installer) fn write(binary: &Path) -> Result<()> {
    registry::set(
        RUN_KEY,
        VALUE_NAME,
        &registry::Value::Text(format_command(binary)),
    )?;
    remove_legacy_startup_file()
}

pub(in crate::installer) fn remove() -> Result<()> {
    registry::delete_value(RUN_KEY, VALUE_NAME)?;
    remove_legacy_startup_file()
}

fn disabled_in_startup_apps() -> Result<bool> {
    let state = read_binary(Hive::CurrentUser, STARTUP_APPROVED_KEY, VALUE_NAME)
        .with_context(|| format!("failed to read HKCU\\{STARTUP_APPROVED_KEY}\\{VALUE_NAME}"))?;
    Ok(state.is_some_and(|state| approval_disabled(&state)))
}

fn approval_disabled(state: &[u8]) -> bool {
    state.first().is_some_and(|flags| flags & 1 == 1)
}

fn format_command(binary: &Path) -> String {
    format!("\"{}\"", binary.display())
}

fn parse_command(command: &str) -> Option<PathBuf> {
    let command = command.trim();
    let program = match command.strip_prefix('"') {
        Some(rest) => &rest[..rest.find('"')?],
        None => command,
    };
    (!program.is_empty()).then(|| PathBuf::from(program))
}

fn legacy_startup_file() -> Option<PathBuf> {
    let app_data = std::env::var_os("APPDATA")?;
    Some(
        PathBuf::from(app_data)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup")
            .join(LEGACY_STARTUP_FILE),
    )
}

fn remove_legacy_startup_file() -> Result<()> {
    let Some(path) = legacy_startup_file() else {
        return Ok(());
    };
    match std::fs::remove_file(&path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error)
            .with_context(|| format!("Failed to remove legacy autostart {}", path.display())),
        _ => Ok(()),
    }
}

fn read_legacy_cmd(path: &Path) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(parse_legacy_start_line(&content).map(PathBuf::from))
}

fn parse_legacy_start_line(content: &str) -> Option<String> {
    let start = content.find("start \"\" \"")? + "start \"\" \"".len();
    let rest = &content[start..];
    let end = rest.rfind('"')?;
    let raw = &rest[..end];
    Some(raw.replace("\"\"", "\"").replace("%%", "%"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_command_round_trips_install_paths() {
        let cases = [
            r"C:\Users\x\AppData\Local\Programs\qol-tray\bin\qol-tray.exe",
            r"C:\Users\x y\repos\qol-tray\target\debug\qol-tray.exe",
            r"C:\Users\x%USERNAME%\qol-tray.exe",
        ];
        for binary in cases {
            let binary = PathBuf::from(binary);
            assert_eq!(parse_command(&format_command(&binary)), Some(binary));
        }
    }

    #[test]
    fn run_command_parse_accepts_unquoted_and_rejects_empty() {
        let cases = [
            (r"C:\qol\qol-tray.exe", Some(r"C:\qol\qol-tray.exe")),
            (
                r#""C:\x y\qol-tray.exe" --flag"#,
                Some(r"C:\x y\qol-tray.exe"),
            ),
            ("\"\"", None),
            ("", None),
            ("\"C:\\unterminated", None),
        ];
        for (command, expected) in cases {
            assert_eq!(
                parse_command(command),
                expected.map(PathBuf::from),
                "{command}"
            );
        }
    }

    #[test]
    fn startup_approved_odd_first_byte_means_disabled() {
        let cases: [(&[u8], bool); 6] = [
            (&[0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], false),
            (&[0x06, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], false),
            (&[0x03, 1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 0], true),
            (&[0x01], true),
            (&[0x07], true),
            (&[], false),
        ];
        for (state, disabled) in cases {
            assert_eq!(approval_disabled(state), disabled, "{state:?}");
        }
    }

    #[test]
    fn legacy_startup_cmd_still_names_its_target() {
        let tmp = tempfile::TempDir::new().unwrap();
        let cmd = tmp.path().join(LEGACY_STARTUP_FILE);
        std::fs::write(
            &cmd,
            "@echo off\r\nsetlocal DisableDelayedExpansion\r\nstart \"\" \"C:\\Users\\x%%USERNAME%%\\qol-tray.exe\"\r\n",
        )
        .unwrap();
        assert_eq!(
            read_legacy_cmd(&cmd).unwrap(),
            Some(PathBuf::from(r"C:\Users\x%USERNAME%\qol-tray.exe"))
        );
        assert_eq!(
            read_legacy_cmd(&tmp.path().join("missing.cmd")).unwrap(),
            None
        );
    }
}
