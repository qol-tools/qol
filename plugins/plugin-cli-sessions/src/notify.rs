use crate::status::Status;
use crate::tool::Tool;

pub struct Notice {
    pub title: String,
    pub body: String,
}

impl Notice {
    pub fn new(tool: Tool, label: String, summary: &str) -> Self {
        let prefix = match tool {
            Tool::Claude => "Claude \u{00B7} ",
            Tool::Codex => "Codex \u{00B7} ",
            Tool::Generic => "",
        };
        Self {
            title: label,
            body: format!("{prefix}{summary}"),
        }
    }
}

pub fn announces_attention(prev: Status, new: Status) -> bool {
    new != prev && new.is_attention()
}

pub fn send(notice: &Notice) {
    let _ = spawn_platform(&notice.title, &notice.body);
}

#[cfg(target_os = "macos")]
fn spawn_platform(title: &str, body: &str) -> std::io::Result<()> {
    let script = format!(
        "display notification {} with title {}",
        applescript_quote(body),
        applescript_quote(title)
    );
    std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|_| ())
}

#[cfg(not(target_os = "macos"))]
fn spawn_platform(title: &str, body: &str) -> std::io::Result<()> {
    std::process::Command::new("notify-send")
        .arg(title)
        .arg(body)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|_| ())
}

#[cfg(target_os = "macos")]
fn applescript_quote(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}
