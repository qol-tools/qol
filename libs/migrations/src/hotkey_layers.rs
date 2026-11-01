use serde_json::Value;

pub const OS_ONLY_FIELD: &str = "os_only";
pub const HOLD_FIELD: &str = "hold_ms";

pub fn merge(core: &[Value], os: &[Value]) -> Vec<Value> {
    core.iter()
        .filter(|shared| !os.iter().any(|local| hides(local, shared)))
        .map(|shared| with_os_only(shared, false))
        .chain(os.iter().map(|local| with_os_only(local, true)))
        .collect()
}

pub fn split(edited: &[Value], old_core: &[Value], old_os: &[Value]) -> (Vec<Value>, Vec<Value>) {
    let (os, shared): (Vec<&Value>, Vec<&Value>) =
        edited.iter().partition(|binding| is_os_only(binding));
    let os = os
        .into_iter()
        .map(|binding| with_os_only(binding, false))
        .collect::<Vec<_>>();
    let mut core = shared
        .into_iter()
        .map(|binding| with_os_only(binding, false))
        .collect::<Vec<_>>();
    let overridden = old_core
        .iter()
        .filter(|shared| {
            old_os.iter().chain(&os).any(|local| hides(local, shared))
                && !core.iter().any(|binding| same_id(binding, shared))
        })
        .cloned()
        .collect::<Vec<_>>();
    core.extend(overridden);
    (core, os)
}

pub fn absorb(core: &mut Vec<Value>, binding: &Value) -> bool {
    let binding = with_os_only(binding, false);
    if core.iter().any(|shared| same_binding(shared, &binding)) {
        return true;
    }
    if core.iter().any(|shared| hides(&binding, shared)) {
        return false;
    }
    core.push(binding);
    true
}

pub fn is_os_only(binding: &Value) -> bool {
    binding
        .get(OS_ONLY_FIELD)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn hides(local: &Value, shared: &Value) -> bool {
    let enabled = local
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    same_id(local, shared)
        || (enabled
            && holds(local) == holds(shared)
            && key(local).is_some_and(|local_key| Some(local_key) == key(shared)))
}

fn holds(binding: &Value) -> bool {
    binding.get(HOLD_FIELD).is_some_and(|hold| !hold.is_null())
}

fn same_id(left: &Value, right: &Value) -> bool {
    text(left, "id").is_some_and(|id| Some(id) == text(right, "id"))
}

fn same_binding(left: &Value, right: &Value) -> bool {
    key(left) == key(right)
        && plugin(left) == plugin(right)
        && text(left, "action") == text(right, "action")
        && left.get("enabled") == right.get("enabled")
        && left.get(HOLD_FIELD) == right.get(HOLD_FIELD)
}

fn key(binding: &Value) -> Option<String> {
    text(binding, "key").map(|key| key.trim().to_ascii_lowercase())
}

fn plugin(binding: &Value) -> Option<&str> {
    text(binding, "plugin_uid").or_else(|| text(binding, "plugin_id"))
}

fn text<'a>(binding: &'a Value, field: &str) -> Option<&'a str> {
    binding.get(field).and_then(Value::as_str)
}

fn with_os_only(binding: &Value, os_only: bool) -> Value {
    let mut binding = binding.clone();
    if let Some(object) = binding.as_object_mut() {
        if os_only {
            object.insert(OS_ONLY_FIELD.into(), Value::Bool(true));
        } else {
            object.remove(OS_ONLY_FIELD);
        }
    }
    binding
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn binding(id: &str, key: &str, action: &str) -> Value {
        json!({ "id": id, "key": key, "plugin_uid": "p", "action": action, "enabled": true })
    }

    fn os_only(value: Value) -> Value {
        with_os_only(&value, true)
    }

    #[test]
    fn merge_lets_an_os_binding_hide_a_shared_one_by_id_or_key() {
        let core = [
            binding("a", "Super+A", "one"),
            binding("b", "Super+B", "two"),
            binding("c", "Super+C", "three"),
        ];
        let os = [
            binding("a", "Super+X", "one"),
            binding("d", "Super+B", "four"),
        ];

        assert_eq!(
            merge(&core, &os),
            vec![
                binding("c", "Super+C", "three"),
                os_only(binding("a", "Super+X", "one")),
                os_only(binding("d", "Super+B", "four")),
            ]
        );
    }

    #[test]
    fn a_disabled_os_binding_hides_a_shared_one_by_id_only() {
        let mut off = binding("a", "Super+A", "one");
        off["enabled"] = json!(false);
        let core = [
            binding("a", "Super+A", "one"),
            binding("b", "Super+A", "two"),
        ];

        assert_eq!(
            merge(&core, std::slice::from_ref(&off)),
            vec![binding("b", "Super+A", "two"), os_only(off)]
        );
    }

    #[test]
    fn split_keeps_a_shared_binding_this_os_overrides() {
        let old_core = [
            binding("a", "Super+A", "one"),
            binding("b", "Super+B", "two"),
        ];
        let edited = [
            os_only(binding("a", "Super+X", "one")),
            binding("b", "Super+B", "two"),
        ];

        let (core, os) = split(&edited, &old_core, &[]);

        assert_eq!(
            core,
            vec![
                binding("b", "Super+B", "two"),
                binding("a", "Super+A", "one")
            ]
        );
        assert_eq!(os, vec![binding("a", "Super+X", "one")]);
    }

    #[test]
    fn split_brings_the_shared_binding_back_when_the_override_is_deleted() {
        let old_core = [binding("a", "Super+A", "one")];
        let old_os = [binding("a", "Super+X", "one")];

        let (core, os) = split(&[], &old_core, &old_os);

        assert_eq!(core, vec![binding("a", "Super+A", "one")]);
        assert!(os.is_empty());
    }

    #[test]
    fn split_removes_a_visible_shared_binding_the_user_deleted() {
        let old_core = [
            binding("a", "Super+A", "one"),
            binding("b", "Super+B", "two"),
        ];

        let (core, _) = split(&[binding("b", "Super+B", "two")], &old_core, &[]);

        assert_eq!(core, vec![binding("b", "Super+B", "two")]);
    }

    #[test]
    fn split_lets_a_shared_edit_replace_the_binding_it_overrode() {
        let old_core = [binding("a", "Super+A", "one")];
        let old_os = [binding("a", "Super+X", "one")];

        let (core, os) = split(&[binding("a", "Super+Y", "one")], &old_core, &old_os);

        assert_eq!(core, vec![binding("a", "Super+Y", "one")]);
        assert!(os.is_empty());
    }

    #[test]
    fn absorb_merges_matches_and_refuses_clashes() {
        let mut core = vec![binding("a", "Super+A", "one")];

        assert!(absorb(&mut core, &binding("other-id", "super+a", "one")));
        assert!(!absorb(&mut core, &binding("b", "Super+A", "two")));
        assert!(!absorb(&mut core, &binding("a", "Super+Z", "one")));
        assert!(absorb(&mut core, &binding("c", "Super+C", "three")));
        assert_eq!(
            core,
            vec![
                binding("a", "Super+A", "one"),
                binding("c", "Super+C", "three")
            ]
        );
    }

    fn with_hold(mut value: Value, hold_ms: u64) -> Value {
        value["hold_ms"] = json!(hold_ms);
        value
    }

    #[test]
    fn merge_keeps_the_hold_time_of_both_layers() {
        let core = [with_hold(binding("a", "Super+A", "one"), 1000)];
        let os = [with_hold(binding("b", "Super+B", "two"), 250)];

        let merged = merge(&core, &os);

        assert_eq!(merged[0]["hold_ms"], json!(1000));
        assert_eq!(merged[1]["hold_ms"], json!(250));
        assert_eq!(merged[1][OS_ONLY_FIELD], json!(true));
    }

    #[test]
    fn split_keeps_the_hold_time_in_the_layer_it_lands_in() {
        let edited = [
            with_hold(binding("a", "Super+A", "one"), 1000),
            os_only(with_hold(binding("b", "Super+B", "two"), 400)),
        ];

        let (core, os) = split(&edited, &[], &[]);

        assert_eq!(core, vec![with_hold(binding("a", "Super+A", "one"), 1000)]);
        assert_eq!(os, vec![with_hold(binding("b", "Super+B", "two"), 400)]);
    }

    #[test]
    fn a_split_then_merge_round_trip_loses_no_hold_time() {
        let edited = vec![
            binding("tap", "Shift+Super+S", "monitor"),
            with_hold(binding("hold", "Shift+Super+S", "region"), 1000),
            os_only(with_hold(binding("local", "Super+L", "lock"), 600)),
        ];

        let (core, os) = split(&edited, &[], &[]);
        let merged = merge(&core, &os);

        let hold_of = |id: &str| {
            merged
                .iter()
                .find(|binding| binding["id"] == id)
                .map(|binding| binding.get("hold_ms").cloned())
        };
        assert_eq!(hold_of("tap"), Some(None));
        assert_eq!(hold_of("hold"), Some(Some(json!(1000))));
        assert_eq!(hold_of("local"), Some(Some(json!(600))));
    }

    #[test]
    fn a_local_tap_never_hides_a_shared_hold_on_the_same_key_and_the_reverse() {
        let shared_hold = with_hold(binding("hold", "Shift+Super+S", "region"), 1000);
        let local_tap = binding("tap", "shift+super+s", "monitor");

        let merged = merge(
            std::slice::from_ref(&shared_hold),
            std::slice::from_ref(&local_tap),
        );
        assert_eq!(
            merged,
            vec![shared_hold.clone(), os_only(local_tap.clone())]
        );

        let shared_tap = binding("tap2", "Shift+Super+S", "monitor");
        let local_hold = with_hold(binding("hold2", "shift+super+s", "region"), 700);
        let merged = merge(
            std::slice::from_ref(&shared_tap),
            std::slice::from_ref(&local_hold),
        );
        assert_eq!(merged, vec![shared_tap, os_only(local_hold)]);
    }

    #[test]
    fn a_local_hold_still_hides_a_shared_hold_on_the_same_key() {
        let shared = with_hold(binding("a", "Shift+Super+S", "region"), 1000);
        let local = with_hold(binding("b", "shift+super+s", "other"), 500);

        assert_eq!(
            merge(std::slice::from_ref(&shared), std::slice::from_ref(&local)),
            vec![os_only(local)]
        );
    }

    #[test]
    fn absorb_treats_a_different_hold_time_as_a_different_binding() {
        let mut core = vec![binding("a", "Super+A", "one")];

        assert!(absorb(
            &mut core,
            &with_hold(binding("b", "Super+A", "one"), 1000)
        ));
        assert_eq!(core.len(), 2);
        assert!(absorb(
            &mut core,
            &with_hold(binding("c", "super+a", "one"), 1000)
        ));
        assert_eq!(core.len(), 2);
    }
}
