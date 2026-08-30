use qol_agent_homes::{Harness, Registry};
use sha2::{Digest, Sha256};

use crate::store::Unit;

pub fn unit_home<'a>(unit: &'a Unit, registry: &'a Registry) -> &'a str {
    if let Some(agent_home) = unit.agent_home.as_deref() {
        return agent_home;
    }
    match unit.source.as_deref() {
        Some("pi") => &registry.default_for(Harness::Pi).id,
        _ => &registry.default_for(Harness::Claude).id,
    }
}

pub fn visible(unit: &Unit, caller: &str, registry: &Registry) -> bool {
    let home = unit_home(unit, registry);
    home == caller || registry.is_shared(home)
}

pub fn cache_slug(caller: &str) -> String {
    let digest = Sha256::digest(caller.as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    hex[..8].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn registry() -> Registry {
        Registry::load_from(None, Path::new("/qol-memory-home-fake"), &|_| None)
    }

    fn unit(source: Option<&str>, agent_home: Option<&str>) -> Unit {
        Unit {
            key: "k".to_string(),
            source: source.map(str::to_owned),
            agent_home: agent_home.map(str::to_owned),
            host: None,
            file: None,
            session: None,
            cwd: None,
            kind: "user".to_string(),
            ts: None,
            text: "text".to_string(),
        }
    }

    #[test]
    fn explicit_home_wins_and_legacy_units_map_by_source() {
        let registry = registry();
        let claude_default = registry.default_for(Harness::Claude).id.clone();
        let pi_default = registry.default_for(Harness::Pi).id.clone();

        assert_eq!(
            unit(Some("pi"), Some("/elsewhere")).agent_home.as_deref(),
            Some("/elsewhere")
        );
        let pinned = unit(Some("pi"), Some("/elsewhere"));
        assert_eq!(unit_home(&pinned, &registry), "/elsewhere");
        assert_eq!(unit_home(&unit(Some("pi"), None), &registry), pi_default);
        assert_eq!(
            unit_home(&unit(Some("claude"), None), &registry),
            claude_default
        );
        assert_eq!(
            unit_home(&unit(Some("agent"), None), &registry),
            claude_default
        );
        assert_eq!(unit_home(&unit(None, None), &registry), claude_default);
    }

    #[test]
    fn visible_accepts_caller_home_and_shared_homes_only() {
        let registry = registry();
        let claude_default = registry.default_for(Harness::Claude).id.clone();
        let pi_default = registry.default_for(Harness::Pi).id.clone();

        assert!(visible(
            &unit(Some("claude"), Some(claude_default.as_str())),
            &claude_default,
            &registry
        ));
        assert!(!visible(
            &unit(Some("claude"), Some("/private-home")),
            &claude_default,
            &registry
        ));
        assert!(visible(
            &unit(Some("pi"), None),
            "unrelated-caller",
            &registry
        ));
        assert!(visible(
            &unit(Some("pi"), Some(pi_default.as_str())),
            "unrelated-caller",
            &registry
        ));
        assert!(registry.is_shared(&pi_default));
        assert!(!registry.is_shared(&claude_default));
    }

    #[test]
    fn cache_slug_is_stable_eight_lowercase_hex_chars() {
        let slug = cache_slug("/home/user/.claude");
        assert_eq!(slug.len(), 8);
        assert!(slug
            .chars()
            .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase()));
        assert_eq!(slug, cache_slug("/home/user/.claude"));
        assert_ne!(slug, cache_slug("/home/user/.claude-work"));
    }
}
