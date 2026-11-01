use crate::hotkeys::HotkeyBinding;
use crate::plugins::PluginUid;

pub(super) fn bound_on_every_key(
    keys: &[String],
    bindings: &[HotkeyBinding],
    uid: &PluginUid,
    action: &str,
    normalize: impl Fn(&str) -> Option<String>,
) -> bool {
    if keys.is_empty() {
        return false;
    }
    keys.iter().all(|key| {
        let Some(wanted) = normalize(key) else {
            return false;
        };
        bindings.iter().any(|binding| {
            binding.enabled
                && binding.hold_ms.is_none()
                && binding.plugin_uid == *uid
                && binding.action == action
                && normalize(&binding.key).as_deref() == Some(wanted.as_str())
        })
    })
}

pub(super) fn is_held(
    holder: Option<&str>,
    plugin_id: &str,
    bound: impl FnOnce() -> bool,
    shadowed: impl FnOnce() -> bool,
) -> bool {
    if holder == Some(plugin_id) {
        return true;
    }
    bound() && !shadowed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn lower(combo: &str) -> Option<String> {
        Some(combo.to_lowercase())
    }

    fn binding(id: &str, key: &str, uid: &str, action: &str, enabled: bool) -> HotkeyBinding {
        HotkeyBinding {
            id: id.to_string(),
            key: key.to_string(),
            plugin_uid: PluginUid::new(uid),
            action: action.to_string(),
            enabled,
            os_only: false,
            hold_ms: None,
        }
    }

    fn keys(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn every_route_key_needs_an_enabled_binding_for_the_declared_action() {
        let uid = PluginUid::new("shot");
        let bindings = [
            binding("a", "Print", "shot", "screenshot", true),
            binding("b", "Shift+Print", "shot", "screenshot", false),
        ];
        assert!(bound_on_every_key(
            &keys(&["print"]),
            &bindings,
            &uid,
            "screenshot",
            lower
        ));
        assert!(!bound_on_every_key(
            &keys(&["print", "shift+print"]),
            &bindings,
            &uid,
            "screenshot",
            lower
        ));
    }

    #[test]
    fn a_hold_binding_does_not_bind_the_tap_key() {
        let uid = PluginUid::new("shot");
        let held = HotkeyBinding {
            hold_ms: Some(1000),
            ..binding("a", "Print", "shot", "screenshot", true)
        };
        assert!(!bound_on_every_key(
            &keys(&["print"]),
            &[held],
            &uid,
            "screenshot",
            lower
        ));
    }

    #[test]
    fn another_plugin_or_action_does_not_count() {
        let uid = PluginUid::new("shot");
        let bindings = [
            binding("a", "Print", "other", "screenshot", true),
            binding("b", "Print", "shot", "record", true),
        ];
        assert!(!bound_on_every_key(
            &keys(&["Print"]),
            &bindings,
            &uid,
            "screenshot",
            lower
        ));
    }

    #[test]
    fn an_empty_route_is_never_bound() {
        assert!(!bound_on_every_key(
            &[],
            &[],
            &PluginUid::new("shot"),
            "screenshot",
            lower
        ));
    }

    #[test]
    fn an_unnormalizable_key_is_not_bound() {
        let uid = PluginUid::new("shot");
        let bindings = [binding("a", "Print", "shot", "screenshot", true)];
        assert!(!bound_on_every_key(
            &keys(&["Print"]),
            &bindings,
            &uid,
            "screenshot",
            |_| None
        ));
    }

    #[test]
    fn the_ledger_holder_is_held_without_looking_further() {
        let looked = Cell::new(false);
        assert!(is_held(
            Some("qol-shot"),
            "qol-shot",
            || {
                looked.set(true);
                false
            },
            || true
        ));
        assert!(!looked.get());
    }

    #[test]
    fn bound_and_unshadowed_is_held_for_a_plugin_that_is_not_the_holder() {
        assert!(is_held(None, "qol-shot", || true, || false));
        assert!(is_held(Some("qol-other"), "qol-shot", || true, || false));
        assert!(!is_held(None, "qol-shot", || true, || true));
        assert!(!is_held(None, "qol-shot", || false, || false));
    }

    #[test]
    fn an_unbound_plugin_never_asks_whether_the_host_shadows() {
        let asked = Cell::new(false);
        assert!(!is_held(
            None,
            "qol-shot",
            || false,
            || {
                asked.set(true);
                false
            }
        ));
        assert!(!asked.get());
    }
}
