use std::process::{Command, Stdio};

use super::NotificationPlatform;

pub(super) struct Platform;

impl NotificationPlatform for Platform {
    fn send_notification(&self, title: &str, message: &str) -> bool {
        send_osascript_notification(title, message)
    }
}

fn send_osascript_notification(title: &str, message: &str) -> bool {
    let script = format!(
        "display notification {} with title {}",
        applescript_quote(message),
        applescript_quote(title)
    );

    Command::new("osascript")
        .arg("-e")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn applescript_quote(input: &str) -> String {
    let escaped = input.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    #[test]
    fn applescript_quote_escapes_quotes_and_backslashes() {
        assert_eq!(
            super::applescript_quote(r#"say "hi" from C:\tmp"#),
            r#""say \"hi\" from C:\\tmp""#
        );
    }
}
