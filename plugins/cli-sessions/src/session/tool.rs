use qol_terminal_sessions::cli::{
    CliSessionDescriptor, CliToolColor, CLAUDE_TOOL_ACCENT, CLAUDE_TOOL_ID, CODEX_TOOL_ACCENT,
    CODEX_TOOL_ID, GENERIC_TOOL_ACCENT,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Tool {
    Claude,
    Codex,
    Generic,
}

impl Tool {
    pub fn from_cli_session(session: &CliSessionDescriptor) -> Self {
        match session.tool.id.as_str() {
            CODEX_TOOL_ID => Self::Codex,
            CLAUDE_TOOL_ID => Self::Claude,
            _ => Self::Generic,
        }
    }

    pub fn accent(self) -> CliToolColor {
        match self {
            Self::Claude => CLAUDE_TOOL_ACCENT,
            Self::Codex => CODEX_TOOL_ACCENT,
            Self::Generic => GENERIC_TOOL_ACCENT,
        }
    }
}
