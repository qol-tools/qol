use std::process::{Command, Stdio};

pub fn send_notification(title: &str, message: &str) {
    if send_platform_notification(title, message) {
        return;
    }

    println!("{title}: {message}");
}

#[cfg(target_os = "macos")]
fn send_platform_notification(title: &str, message: &str) -> bool {
    send_osascript_notification(title, message) || send_notify_send_notification(title, message)
}

#[cfg(not(target_os = "macos"))]
fn send_platform_notification(title: &str, message: &str) -> bool {
    send_notify_send_notification(title, message)
}

#[cfg(target_os = "macos")]
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

fn send_notify_send_notification(title: &str, message: &str) -> bool {
    Command::new("notify-send")
        .arg(title)
        .arg(message)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn applescript_quote(input: &str) -> String {
    let escaped = input.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    #[test]
    fn applescript_quote_escapes_quotes_and_backslashes() {
        assert_eq!(
            super::applescript_quote(r#"say "hi" from C:\tmp"#),
            r#""say \"hi\" from C:\\tmp""#
        );
    }
}
