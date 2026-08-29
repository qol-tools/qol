mod claude;
mod codex;
mod generic;
mod kimi;
mod pi;
pub use pi::{session_file_containing_marker, session_file_for_external_id};

use std::sync::Arc;

use crate::cli::{CliSessionStrategy, CliTool, CliToolColor, CliToolId};

pub(in crate::cli) use generic::GenericStrategy;

pub const GENERIC_TOOL_ID: &str = "generic";
pub const CODEX_TOOL_ID: &str = "codex";
pub const CLAUDE_TOOL_ID: &str = "claude";
pub const PI_TOOL_ID: &str = "pi";
pub const KIMI_TOOL_ID: &str = "kimi";

pub const GENERIC_TOOL_ACCENT: CliToolColor = CliToolColor::new(0x87, 0x92, 0xa8);
pub const CODEX_TOOL_ACCENT: CliToolColor = CliToolColor::new(0x82, 0xaa, 0xff);
pub const CLAUDE_TOOL_ACCENT: CliToolColor = CliToolColor::new(0xf0, 0xa2, 0x7a);
pub const PI_TOOL_ACCENT: CliToolColor = CliToolColor::new(0x8a, 0xbe, 0xb7);
pub const KIMI_TOOL_ACCENT: CliToolColor = CliToolColor::new(0x4f, 0xa8, 0xff);

pub fn generic_tool() -> CliTool {
    CliTool::new(valid_id(GENERIC_TOOL_ID), "CLI", GENERIC_TOOL_ACCENT)
}

pub fn codex_tool() -> CliTool {
    CliTool::new(valid_id(CODEX_TOOL_ID), "Codex", CODEX_TOOL_ACCENT)
}

pub fn claude_tool() -> CliTool {
    CliTool::new(valid_id(CLAUDE_TOOL_ID), "Claude", CLAUDE_TOOL_ACCENT)
}

pub fn pi_tool() -> CliTool {
    CliTool::new(valid_id(PI_TOOL_ID), "Pi", PI_TOOL_ACCENT)
}

pub fn kimi_tool() -> CliTool {
    CliTool::new(valid_id(KIMI_TOOL_ID), "Kimi", KIMI_TOOL_ACCENT)
}

pub(super) fn system_strategies() -> [Arc<dyn CliSessionStrategy>; 4] {
    [
        Arc::new(codex::CodexStrategy::default()),
        Arc::new(claude::ClaudeStrategy::default()),
        Arc::new(pi::PiStrategy::default()),
        Arc::new(kimi::KimiStrategy::default()),
    ]
}

pub(super) fn fallback_name(session: &crate::SessionFacts) -> Option<String> {
    session
        .spawn_identity
        .as_ref()
        .map(|identity| identity.key.as_str().to_owned())
        .or_else(|| project_name(&session.cwd))
}

pub(super) fn project_name(cwd: &str) -> Option<String> {
    std::path::Path::new(cwd)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn valid_id(id: &'static str) -> CliToolId {
    CliToolId::new(id).expect("built-in CLI tool ids are valid")
}

#[cfg(test)]
mod tests {
    use super::{claude_tool, codex_tool, generic_tool, kimi_tool, pi_tool};

    #[test]
    fn built_in_tools_own_distinct_pastel_accents() {
        let cases = [
            (generic_tool(), 0x8792a8),
            (codex_tool(), 0x82aaff),
            (claude_tool(), 0xf0a27a),
            (pi_tool(), 0x8abeb7),
            (kimi_tool(), 0x4fa8ff),
        ];

        for (tool, expected) in cases {
            assert_eq!(tool.accent.rgb24(), expected, "tool: {}", tool.id);
        }
    }
}
