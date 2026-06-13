use crate::manifest::{PluginId, PluginInfo};
use anyhow::{anyhow, bail, Result};

impl PluginInfo {
    pub fn validate_identity(&self) -> Result<()> {
        match &self.id {
            Some(id) => validate_plugin_id(id.as_str()),
            None => Ok(()),
        }
    }

    pub fn require_declared_id(&self) -> Result<&PluginId> {
        let id = self.id.as_ref().ok_or_else(|| {
            anyhow!("plugin.toml is missing required [plugin].id - add id = \"...\" under [plugin]")
        })?;
        validate_plugin_id(id.as_str())?;
        Ok(id)
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
    use crate::manifest::PluginInfo;

    fn plugin_info(id: Option<&str>) -> PluginInfo {
        PluginInfo {
            id: id.map(Into::into),
            name: "n".into(),
            description: String::new(),
            version: "1.0.0".into(),
            author: None,
            platforms: None,
        }
    }

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

    #[test]
    fn validate_identity_tolerates_absent_id() {
        assert!(
            plugin_info(None).validate_identity().is_ok(),
            "an absent id is tolerated at load; identity comes from the locator"
        );
    }

    #[test]
    fn require_declared_id_enforces_presence_and_charset() {
        assert!(
            plugin_info(None).require_declared_id().is_err(),
            "absent id must be rejected at the authority boundary"
        );
        assert!(
            plugin_info(Some("bad id")).require_declared_id().is_err(),
            "invalid charset must be rejected"
        );
        assert_eq!(
            plugin_info(Some("plugin-ok"))
                .require_declared_id()
                .unwrap()
                .as_str(),
            "plugin-ok"
        );
    }
}
