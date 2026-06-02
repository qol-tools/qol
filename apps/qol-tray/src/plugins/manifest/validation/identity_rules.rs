use crate::plugins::manifest::PluginInfo;
use anyhow::{bail, Result};

impl PluginInfo {
    pub fn validate_identity(&self) -> Result<()> {
        validate_plugin_id(self.id.as_str())
    }
}

pub fn is_valid_plugin_id(value: &str) -> bool {
    super::command_rules::is_valid_command_basename(value)
}

pub(super) fn validate_plugin_id(value: &str) -> Result<()> {
    if is_valid_plugin_id(value) {
        return Ok(());
    }

    bail!(
        "plugin.id {value:?} must be non-empty, at most 64 chars, not start with '-', \
         and contain only [A-Za-z0-9_-]"
    )
}

#[cfg(test)]
mod tests {
    use super::is_valid_plugin_id;

    #[test]
    fn accepts_safe_ids() {
        let cases = ["plugin-alt-tab", "alt_tab", "keyremap", "a", "A1-b_2"];
        for id in cases {
            assert!(is_valid_plugin_id(id), "should accept: {id}");
        }
    }

    #[test]
    fn rejects_unsafe_ids() {
        let cases = [
            ("", "empty"),
            ("-leading", "leading dash"),
            ("has space", "space"),
            ("dot.dot", "dot"),
            ("slash/inside", "path separator"),
            ("..", "parent dir"),
        ];
        for (id, why) in cases {
            assert!(!is_valid_plugin_id(id), "should reject ({why}): {id:?}");
        }
    }

    #[test]
    fn rejects_overlong_id() {
        let id = "a".repeat(65);
        assert!(!is_valid_plugin_id(&id), "65 chars should be rejected");
    }
}
