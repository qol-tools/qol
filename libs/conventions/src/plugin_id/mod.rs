mod types;

pub use types::{PluginId, PluginUid};

pub fn is_valid_plugin_uid(value: &str) -> bool {
    !value.trim().is_empty() && value.trim() == value && !value.chars().any(char::is_control)
}

pub fn short_name(id: &str) -> &str {
    id.strip_prefix("qol-")
        .or_else(|| id.strip_prefix("plugin-"))
        .unwrap_or(id)
}

pub fn matches(id: &str, query: &str) -> bool {
    id == query || short_name(id) == short_name(query)
}

#[cfg(test)]
mod tests {
    use super::{is_valid_plugin_uid, matches, short_name, PluginId, PluginUid};

    #[test]
    fn uid_syntax_preserves_non_uuid_text_and_rejects_whitespace_and_controls() {
        for (value, valid) in [
            ("", false),
            ("uid", true),
            ("not a uuid", true),
            ("é/值", true),
            (" uid", false),
            ("uid\u{2003}", false),
            ("\u{2003}", false),
            ("u\0id", false),
            ("u\nid", false),
            ("u\u{7f}id", false),
        ] {
            assert_eq!(is_valid_plugin_uid(value), valid, "{value:?}");
        }
    }

    #[test]
    fn identity_newtypes_preserve_string_values_without_normalization() {
        for text in ["", "uid", " id ", "not/a/uuid", "é\n值"] {
            let id = PluginId::new(text);
            let uid = PluginUid::new(text);
            let wire = serde_json::Value::String(text.to_string());
            assert_eq!(serde_json::to_value(&id).unwrap(), wire, "id {text:?}");
            assert_eq!(serde_json::to_value(&uid).unwrap(), wire, "uid {text:?}");
            assert_eq!(
                serde_json::from_value::<PluginId>(wire.clone()).unwrap(),
                id,
                "{text:?}"
            );
            assert_eq!(
                serde_json::from_value::<PluginUid>(wire).unwrap(),
                uid,
                "{text:?}"
            );
            assert_eq!(id.to_string(), text, "id {text:?}");
            assert_eq!(uid.to_string(), text, "uid {text:?}");
        }
        for wire in [
            serde_json::json!(null),
            serde_json::json!(17),
            serde_json::json!({}),
        ] {
            assert!(
                serde_json::from_value::<PluginId>(wire.clone()).is_err(),
                "{wire}"
            );
            assert!(
                serde_json::from_value::<PluginUid>(wire.clone()).is_err(),
                "{wire}"
            );
        }
    }

    #[test]
    fn short_name_strips_the_qol_prefix() {
        assert_eq!(short_name("qol-lights"), "lights");
    }

    #[test]
    fn short_name_strips_the_plugin_prefix() {
        assert_eq!(short_name("plugin-lights"), "lights");
    }

    #[test]
    fn short_name_keeps_a_bare_name() {
        assert_eq!(short_name("lights"), "lights");
    }

    #[test]
    fn matches_accepts_every_name_form_of_one_plugin() {
        for id in ["qol-lights", "plugin-lights", "lights"] {
            for query in ["lights", "qol-lights", "plugin-lights"] {
                assert!(matches(id, query), "id={id} query={query}");
            }
        }
    }

    #[test]
    fn matches_rejects_a_different_plugin() {
        assert!(!matches("qol-lights", "qol-launcher"));
        assert!(!matches("qol-lights", "light"));
    }
}
