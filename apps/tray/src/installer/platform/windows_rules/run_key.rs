use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const LEGACY_STARTUP_FILE: &str = "qol-tray.cmd";
const LEGACY_START_PREFIX: &str = "start \"\" \"";

pub(in crate::installer) fn approval_disabled(state: &[u8]) -> bool {
    state.first().is_some_and(|flags| flags & 1 == 1)
}

pub(in crate::installer) fn format_command(binary: &Path) -> String {
    format!("\"{}\"", binary.display())
}

pub(in crate::installer) fn parse_command(command: &str) -> Option<PathBuf> {
    let command = command.trim();
    let program = match command.strip_prefix('"') {
        Some(rest) => &rest[..rest.find('"')?],
        None => command,
    };
    (!program.is_empty()).then(|| PathBuf::from(program))
}

pub(in crate::installer) fn legacy_startup_file(app_data: &Path) -> PathBuf {
    app_data
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Startup")
        .join(LEGACY_STARTUP_FILE)
}

pub(in crate::installer) fn remove_legacy_startup_file(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error)
            .with_context(|| format!("Failed to remove legacy autostart {}", path.display())),
        _ => Ok(()),
    }
}

pub(in crate::installer) fn read_legacy_cmd(path: &Path) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(parse_legacy_start_line(&content).map(PathBuf::from))
}

fn parse_legacy_start_line(content: &str) -> Option<String> {
    let start = content.find(LEGACY_START_PREFIX)? + LEGACY_START_PREFIX.len();
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
            assert_eq!(
                parse_command(&format_command(&binary)),
                Some(binary.clone()),
                "{}",
                binary.display()
            );
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
            (r#"  "C:\x\qol-tray.exe"  "#, Some(r"C:\x\qol-tray.exe")),
            ("\"\"", None),
            ("", None),
            ("   ", None),
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
    fn legacy_start_line_unescapes_cmd_quoting() {
        let cases = [
            (
                "@echo off\r\nstart \"\" \"C:\\Users\\x%%USERNAME%%\\qol-tray.exe\"\r\n",
                Some(r"C:\Users\x%USERNAME%\qol-tray.exe"),
            ),
            (
                "start \"\" \"C:\\a \"\"b\"\"\\qol-tray.exe\"",
                Some(r#"C:\a "b"\qol-tray.exe"#),
            ),
            ("@echo off\r\n", None),
            ("start \"\" \"C:\\unterminated", None),
        ];
        for (content, expected) in cases {
            assert_eq!(
                parse_legacy_start_line(content).as_deref(),
                expected,
                "{content:?}"
            );
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

    #[test]
    fn legacy_startup_file_is_removed_from_the_app_data_startup_folder() {
        let app_data = tempfile::TempDir::new().unwrap();
        let legacy = legacy_startup_file(app_data.path());
        let startup = legacy.parent().unwrap();
        std::fs::create_dir_all(startup).unwrap();
        std::fs::write(&legacy, "@echo off\r\n").unwrap();
        let sibling = startup.join("other.cmd");
        std::fs::write(&sibling, "@echo off\r\n").unwrap();

        remove_legacy_startup_file(&legacy).unwrap();
        remove_legacy_startup_file(&legacy).unwrap();

        assert!(!legacy.exists());
        assert!(
            sibling.exists(),
            "only the qol-tray startup file is removed"
        );
        assert_eq!(
            legacy.strip_prefix(app_data.path()).unwrap(),
            Path::new("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs")
                .join("Startup")
                .join(LEGACY_STARTUP_FILE)
        );
    }
}
