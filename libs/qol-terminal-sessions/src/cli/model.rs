use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};

use crate::cli::CliSessionEvidence;
use crate::IdentityError;

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct CliToolId(String);

impl CliToolId {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        if valid_tool_id(&value) {
            return Ok(Self(value));
        }
        Err(IdentityError::component("CLI tool", value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for CliToolId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliTool {
    pub id: CliToolId,
    pub label: String,
    pub accent: CliToolColor,
}

impl CliTool {
    pub fn new(id: CliToolId, label: impl Into<String>, accent: CliToolColor) -> Self {
        Self {
            id,
            label: label.into(),
            accent,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CliToolColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl CliToolColor {
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self { red, green, blue }
    }

    pub const fn rgb24(self) -> u32 {
        ((self.red as u32) << 16) | ((self.green as u32) << 8) | self.blue as u32
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CliSessionDescriptor {
    pub tool: CliTool,
    pub display_name: Option<String>,
    pub external_id: Option<String>,
    pub has_activity: Option<bool>,
    pub evidence: CliSessionEvidence,
}

pub(crate) fn normalize_display_name(value: Option<String>) -> Option<String> {
    let value = value?;
    let value = value.trim();
    if value.is_empty() || value.chars().any(|character| character.is_control()) {
        return None;
    }
    Some(value.to_owned())
}

fn valid_tool_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::normalize_display_name;

    #[test]
    fn display_names_are_trimmed_and_control_free() {
        assert_eq!(
            normalize_display_name(Some("  project  ".into())),
            Some("project".into())
        );
        assert_eq!(normalize_display_name(Some("\u{1}".into())), None);
        assert_eq!(normalize_display_name(Some("project\u{1}".into())), None);
        assert_eq!(normalize_display_name(Some(" \t ".into())), None);
    }
}
