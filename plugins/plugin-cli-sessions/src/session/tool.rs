use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
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
