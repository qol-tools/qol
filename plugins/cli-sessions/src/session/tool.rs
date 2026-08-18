use qol_terminal_sessions::cli::{CliSessionDescriptor, CliTool, GENERIC_TOOL_ID};

pub type Tool = CliTool;

pub fn from_cli_session(session: &CliSessionDescriptor) -> Tool {
    session.tool.clone()
}

pub fn is_generic(tool: &Tool) -> bool {
    tool.id.as_str() == GENERIC_TOOL_ID
}
