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
    use super::{matches, short_name};

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
