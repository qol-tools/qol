use std::time::Duration;

use serde::{Deserialize, Serialize};

pub mod engine;
pub mod lexer;

pub const HOT_TO_WARM_AFTER: Duration = Duration::from_secs(60);
pub const WARM_TO_COOL_AFTER: Duration = Duration::from_secs(300);

pub fn decayed_heat(heat: HeatLevel, elapsed: Duration) -> HeatLevel {
    match heat {
        HeatLevel::Cool => HeatLevel::Cool,
        HeatLevel::Warm => {
            if elapsed >= WARM_TO_COOL_AFTER {
                HeatLevel::Cool
            } else {
                HeatLevel::Warm
            }
        }
        HeatLevel::Hot => {
            if elapsed >= HOT_TO_WARM_AFTER {
                if elapsed >= WARM_TO_COOL_AFTER {
                    HeatLevel::Cool
                } else {
                    HeatLevel::Warm
                }
            } else {
                HeatLevel::Hot
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DiffStatus {
    Added,
    #[default]
    Modified,
    Deleted,
    Renamed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeatLevel {
    Cool,
    Warm,
    Hot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TokenKind {
    #[default]
    Plain,
    String,
    Comment,
    Keyword,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenSpan {
    pub start: usize,
    pub len: usize,
    pub heat: HeatLevel,
    pub kind: TokenKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineChange {
    pub kind: LineKind,
    pub text: String,
    pub token_spans: Vec<TokenSpan>,
    pub old_line_no: Option<u32>,
    pub new_line_no: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<LineChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FileDiff {
    pub old_path: String,
    pub new_path: String,
    pub status: DiffStatus,
    pub hunks: Vec<Hunk>,
}

impl FileDiff {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffError {
    Binary,
    Encoding,
    Conflict,
    Other,
}

impl std::fmt::Display for DiffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiffError::Binary => write!(f, "file content is binary"),
            DiffError::Encoding => write!(f, "file content is not valid UTF-8"),
            DiffError::Conflict => write!(f, "diff contains unresolved conflict markers"),
            DiffError::Other => write!(f, "diff could not be produced"),
        }
    }
}

impl std::error::Error for DiffError {}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        decayed_heat, DiffError, DiffStatus, FileDiff, HeatLevel, Hunk, LineChange, LineKind,
        TokenKind, TokenSpan,
    };

    #[test]
    fn empty_diff_has_no_hunks() {
        let diff = FileDiff::empty();
        assert!(diff.is_empty());
        assert_eq!(diff.hunks, Vec::new());
    }

    #[test]
    fn model_round_trips_through_json() {
        let diff = FileDiff {
            old_path: "src/a.rs".to_string(),
            new_path: "src/a.rs".to_string(),
            status: DiffStatus::Modified,
            hunks: vec![Hunk {
                old_start: 1,
                old_lines: 3,
                new_start: 1,
                new_lines: 4,
                lines: vec![
                    LineChange {
                        kind: LineKind::Context,
                        text: "fn main() {".to_string(),
                        token_spans: vec![TokenSpan {
                            start: 4,
                            len: 4,
                            heat: HeatLevel::Cool,
                            kind: TokenKind::Plain,
                        }],
                        old_line_no: Some(1),
                        new_line_no: Some(1),
                    },
                    LineChange {
                        kind: LineKind::Added,
                        text: "    println!(\"hi\");".to_string(),
                        token_spans: Vec::new(),
                        old_line_no: None,
                        new_line_no: Some(2),
                    },
                ],
            }],
        };
        let json = serde_json::to_string(&diff).expect("serialize");
        let restored: FileDiff = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, diff);
    }

    #[test]
    fn line_change_carries_gutter_line_numbers() {
        let removed = LineChange {
            kind: LineKind::Removed,
            text: "old".to_string(),
            token_spans: Vec::new(),
            old_line_no: Some(7),
            new_line_no: None,
        };
        assert_eq!(removed.old_line_no, Some(7));
        assert_eq!(removed.new_line_no, None);
        let added = LineChange {
            kind: LineKind::Added,
            text: "new".to_string(),
            token_spans: Vec::new(),
            old_line_no: None,
            new_line_no: Some(8),
        };
        assert_eq!(added.old_line_no, None);
        assert_eq!(added.new_line_no, Some(8));
    }

    #[test]
    fn diff_error_displays_and_serializes() {
        assert_eq!(DiffError::Binary.to_string(), "file content is binary");
        let json = serde_json::to_string(&DiffError::Conflict).expect("serialize");
        assert_eq!(json, "\"Conflict\"");
    }

    #[test]
    fn decay_keeps_fresh_heat_untouched() {
        assert_eq!(
            decayed_heat(HeatLevel::Cool, Duration::ZERO),
            HeatLevel::Cool
        );
        assert_eq!(
            decayed_heat(HeatLevel::Warm, Duration::ZERO),
            HeatLevel::Warm
        );
        assert_eq!(decayed_heat(HeatLevel::Hot, Duration::ZERO), HeatLevel::Hot);
    }

    #[test]
    fn hot_cools_to_warm_after_one_minute() {
        assert_eq!(
            decayed_heat(HeatLevel::Hot, Duration::from_secs(59)),
            HeatLevel::Hot
        );
        assert_eq!(
            decayed_heat(HeatLevel::Hot, Duration::from_secs(60)),
            HeatLevel::Warm
        );
        assert_eq!(
            decayed_heat(HeatLevel::Hot, Duration::from_secs(299)),
            HeatLevel::Warm
        );
    }

    #[test]
    fn warm_cools_to_cool_after_five_minutes() {
        assert_eq!(
            decayed_heat(HeatLevel::Warm, Duration::from_secs(299)),
            HeatLevel::Warm
        );
        assert_eq!(
            decayed_heat(HeatLevel::Warm, Duration::from_secs(300)),
            HeatLevel::Cool
        );
    }

    #[test]
    fn hot_cools_straight_to_cool_after_five_minutes() {
        assert_eq!(
            decayed_heat(HeatLevel::Hot, Duration::from_secs(300)),
            HeatLevel::Cool
        );
        assert_eq!(
            decayed_heat(HeatLevel::Hot, Duration::from_secs(3600)),
            HeatLevel::Cool
        );
    }

    #[test]
    fn cool_never_warms() {
        assert_eq!(
            decayed_heat(HeatLevel::Cool, Duration::from_secs(86_400)),
            HeatLevel::Cool
        );
    }
}
