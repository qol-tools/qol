pub(crate) const INVALID_PLUGIN_ID: &str = "Invalid plugin ID";

pub(crate) fn is_safe_plugin_id(plugin_id: &str) -> bool {
    crate::paths::is_safe_path_component(plugin_id)
}

pub(crate) fn validate_plugin_id(plugin_id: &str) -> Result<(), &'static str> {
    if is_safe_plugin_id(plugin_id) {
        return Ok(());
    }
    Err(INVALID_PLUGIN_ID)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_plugin_id_accepts_safe_values() {
        assert!(validate_plugin_id("plugin-abc").is_ok());
        assert!(validate_plugin_id("plugin_abc").is_ok());
        assert!(validate_plugin_id("PLUGIN123").is_ok());
    }

    #[test]
    fn validate_plugin_id_rejects_unsafe_values() {
        assert_eq!(validate_plugin_id(""), Err(INVALID_PLUGIN_ID));
        assert_eq!(validate_plugin_id("../bad"), Err(INVALID_PLUGIN_ID));
        assert_eq!(validate_plugin_id("bad/path"), Err(INVALID_PLUGIN_ID));
        assert_eq!(validate_plugin_id("-bad"), Err(INVALID_PLUGIN_ID));
    }
}
