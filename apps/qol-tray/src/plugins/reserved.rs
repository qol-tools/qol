const RESERVED_PLUGIN_IDS: &[&str] = &["plugin-template"];

pub fn is_reserved_plugin_id(id: &str) -> bool {
    RESERVED_PLUGIN_IDS.contains(&id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_is_reserved_real_plugins_are_not() {
        let cases = [
            ("plugin-template", true),
            ("plugin-alt-tab", false),
            ("plugin-launcher", false),
            ("", false),
        ];
        for (id, expected) in cases {
            assert_eq!(is_reserved_plugin_id(id), expected, "id: {id}");
        }
    }
}
