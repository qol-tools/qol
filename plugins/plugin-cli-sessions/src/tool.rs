use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tool {
    Claude,
    Codex,
    Generic,
}

pub fn classify(foreground_basenames: &[String]) -> Tool {
    if foreground_basenames.iter().any(|n| n == "codex") {
        Tool::Codex
    } else if foreground_basenames.iter().any(|n| n == "claude") {
        Tool::Claude
    } else {
        Tool::Generic
    }
}
