use serde_json::Value;

pub const OS_ONLY_FIELD: &str = "os_only";

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
    same_id(local, shared) || key(local).is_some_and(|local_key| Some(local_key) == key(shared))
}

fn same_id(left: &Value, right: &Value) -> bool {
    text(left, "id").is_some_and(|id| Some(id) == text(right, "id"))
}

fn same_binding(left: &Value, right: &Value) -> bool {
    key(left) == key(right)
        && plugin(left) == plugin(right)
        && text(left, "action") == text(right, "action")
        && left.get("enabled") == right.get("enabled")
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
}
