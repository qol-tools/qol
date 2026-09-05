use qol_terminal_sessions::cli::{CliSessionDescriptor, CliTool, GENERIC_TOOL_ID};

pub type Tool = CliTool;

pub fn from_cli_session(session: &CliSessionDescriptor) -> Tool {
    session.tool.clone()
}

pub fn is_generic(tool: &Tool) -> bool {
    tool.id.as_str() == GENERIC_TOOL_ID
}

pub fn completion_policy(tool: &Tool) -> crate::attention::CompletionPolicy {
    use qol_terminal_sessions::cli::{CLAUDE_TOOL_ID, CODEX_TOOL_ID, PI_TOOL_ID};
    match tool.id.as_str() {
        CLAUDE_TOOL_ID | CODEX_TOOL_ID | PI_TOOL_ID => crate::attention::CompletionPolicy::Explicit,
        _ => crate::attention::CompletionPolicy::Quiescent,
    }
}
