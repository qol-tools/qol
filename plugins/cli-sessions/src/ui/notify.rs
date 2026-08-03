use crate::session::status::Status;
use crate::session::tool::Tool;

pub struct Notice {
    pub title: String,
    pub body: String,
}

impl Notice {
    pub fn new(tool: Tool, label: String, summary: &str) -> Self {
        let prefix = match tool {
            Tool::Claude => "Claude \u{00B7} ",
            Tool::Codex => "Codex \u{00B7} ",
            Tool::Kimi => "Kimi \u{00B7} ",
            Tool::Pi => "Pi \u{00B7} ",
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
    qol_plugin_daemon::notification::send_notification(&notice.title, &notice.body);
}
