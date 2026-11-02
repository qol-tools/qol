use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::compile::{Compiled, RULE_DESCRIPTION};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Sidecar {
    #[serde(default)]
    pub(crate) profile: Option<String>,
    #[serde(default)]
    pub(crate) simple_modifications: Vec<String>,
    #[serde(default)]
    pub(crate) block_hash: Option<String>,
}

#[derive(Debug)]
pub(crate) struct Synced {
    pub(crate) document: Value,
    pub(crate) sidecar: Sidecar,
    pub(crate) profile: String,
    pub(crate) changed: bool,
    pub(crate) edited_by_hand: bool,
    pub(crate) conflicts: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct Removed {
    pub(crate) document: Value,
    pub(crate) changed: bool,
}

pub(crate) fn sync(document: &Value, compiled: &Compiled, sidecar: &Sidecar) -> Result<Synced> {
    let mut updated = document.clone();
    let Some(profiles) = updated.get_mut("profiles").and_then(Value::as_array_mut) else {
        bail!("karabiner.json has no profiles; open Karabiner-Elements once so it writes one");
    };
    let Some(selected) = profiles
        .iter()
        .position(|profile| profile.get("selected").and_then(Value::as_bool) == Some(true))
    else {
        bail!("karabiner.json has no selected profile; pick one in Karabiner-Elements");
    };
    let name = profile_name(&profiles[selected]);
    let mut edited_by_hand = false;
    for (index, profile) in profiles.iter_mut().enumerate() {
        if index != selected {
            remove_block(profile);
            if sidecar.profile.as_deref() == Some(profile_name(profile).as_str()) {
                remove_simple(profile, &sidecar.simple_modifications);
            }
        }
    }
    let profile = &mut profiles[selected];
    if let (Some(previous), Some(recorded)) = (find_block(profile), &sidecar.block_hash) {
        edited_by_hand = &hash(previous) != recorded;
    }
    put_block(profile, &compiled.rule);
    if sidecar.profile.as_deref() == Some(name.as_str()) {
        remove_simple(profile, &sidecar.simple_modifications);
    }
    let (written, conflicts) = put_simple(profile, &compiled.simple_modifications);
    let changed = updated != *document;
    Ok(Synced {
        document: updated,
        sidecar: Sidecar {
            profile: Some(name.clone()),
            simple_modifications: written,
            block_hash: Some(hash(&compiled.rule)),
        },
        profile: name,
        changed,
        edited_by_hand,
        conflicts,
    })
}

pub(crate) fn remove(document: &Value, sidecar: &Sidecar) -> Removed {
    let mut updated = document.clone();
    if let Some(profiles) = updated.get_mut("profiles").and_then(Value::as_array_mut) {
        for profile in profiles {
            remove_block(profile);
            if sidecar.profile.as_deref() == Some(profile_name(profile).as_str()) {
                remove_simple(profile, &sidecar.simple_modifications);
            }
        }
    }
    let changed = updated != *document;
    Removed {
        document: updated,
        changed,
    }
}

fn profile_name(profile: &Value) -> String {
    profile
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn is_ours(rule: &Value) -> bool {
    rule.get("description").and_then(Value::as_str) == Some(RULE_DESCRIPTION)
}

fn find_block(profile: &Value) -> Option<&Value> {
    profile
        .pointer("/complex_modifications/rules")?
        .as_array()?
        .iter()
        .find(|rule| is_ours(rule))
}

fn remove_block(profile: &mut Value) {
    if let Some(rules) = profile
        .pointer_mut("/complex_modifications/rules")
        .and_then(Value::as_array_mut)
    {
        rules.retain(|rule| !is_ours(rule));
    }
}

fn put_block(profile: &mut Value, block: &Value) {
    let rules = child_array(child_object(profile, "complex_modifications"), "rules");
    match rules.iter().position(is_ours) {
        Some(index) => {
            rules[index] = block.clone();
            let mut seen = false;
            rules.retain(|rule| {
                if !is_ours(rule) {
                    return true;
                }
                let first = !seen;
                seen = true;
                first
            });
        }
        None => rules.push(block.clone()),
    }
}

fn from_key(entry: &Value) -> Option<&str> {
    entry.pointer("/from/key_code").and_then(Value::as_str)
}

fn remove_simple(profile: &mut Value, ours: &[String]) {
    if let Some(entries) = profile
        .get_mut("simple_modifications")
        .and_then(Value::as_array_mut)
    {
        entries.retain(|entry| !from_key(entry).is_some_and(|key| ours.iter().any(|o| o == key)));
    }
}

fn put_simple(profile: &mut Value, wanted: &[Value]) -> (Vec<String>, Vec<String>) {
    let entries = child_array(profile, "simple_modifications");
    let mut written = Vec::new();
    let mut conflicts = Vec::new();
    for entry in wanted {
        let Some(key) = from_key(entry) else {
            continue;
        };
        if entries
            .iter()
            .any(|existing| from_key(existing) == Some(key))
        {
            conflicts.push(key.to_string());
            continue;
        }
        entries.push(entry.clone());
        written.push(key.to_string());
    }
    (written, conflicts)
}

fn child_object<'a>(parent: &'a mut Value, key: &str) -> &'a mut Value {
    if !parent.get(key).is_some_and(Value::is_object) {
        parent[key] = json!({});
    }
    &mut parent[key]
}

fn child_array<'a>(parent: &'a mut Value, key: &str) -> &'a mut Vec<Value> {
    if !parent.get(key).is_some_and(Value::is_array) {
        parent[key] = json!([]);
    }
    parent[key].as_array_mut().expect("just made an array")
}

fn hash(block: &Value) -> String {
    Sha256::digest(block.to_string().as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compiled(manipulators: usize, simple: &[(&str, &str)]) -> Compiled {
        let manipulators: Vec<Value> = (0..manipulators)
            .map(|index| json!({ "type": "basic", "from": { "key_code": format!("f{}", index + 1) } }))
            .collect();
        Compiled {
            rule: json!({ "description": RULE_DESCRIPTION, "manipulators": manipulators }),
            simple_modifications: simple
                .iter()
                .map(|(from, to)| json!({ "from": { "key_code": from }, "to": [{ "key_code": to }] }))
                .collect(),
            gaps: Vec::new(),
        }
    }

    fn karabiner() -> Value {
        json!({
            "global": { "show_in_menu_bar": false },
            "profiles": [
                {
                    "name": "Default profile",
                    "selected": true,
                    "virtual_hid_keyboard": { "keyboard_type_v2": "iso" },
                    "complex_modifications": {
                        "parameters": { "basic.to_if_alone_timeout_milliseconds": 1000 },
                        "rules": [{ "description": "my own rule", "manipulators": [] }]
                    },
                    "simple_modifications": [
                        { "from": { "key_code": "right_command" }, "to": [{ "key_code": "right_option" }] }
                    ]
                },
                { "name": "Gaming", "selected": false }
            ]
        })
    }

    fn rules(document: &Value, profile: usize) -> Vec<Value> {
        document["profiles"][profile]["complex_modifications"]["rules"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    fn select(document: &mut Value, profile: usize) {
        for (index, entry) in document["profiles"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .enumerate()
        {
            entry["selected"] = json!(index == profile);
        }
    }

    #[test]
    fn foreign_rules_and_unknown_fields_survive() {
        let synced = sync(&karabiner(), &compiled(2, &[]), &Sidecar::default()).unwrap();
        let document = &synced.document;
        assert!(synced.changed);
        assert_eq!(synced.profile, "Default profile");
        assert_eq!(document["global"], json!({ "show_in_menu_bar": false }));
        assert_eq!(
            document["profiles"][0]["virtual_hid_keyboard"]["keyboard_type_v2"],
            "iso"
        );
        assert_eq!(
            document["profiles"][0]["complex_modifications"]["parameters"],
            karabiner()["profiles"][0]["complex_modifications"]["parameters"]
        );
        let rules = rules(document, 0);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0]["description"], "my own rule");
        assert_eq!(rules[1]["manipulators"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn our_block_is_replaced_not_duplicated() {
        let first = sync(&karabiner(), &compiled(2, &[]), &Sidecar::default()).unwrap();
        let second = sync(&first.document, &compiled(5, &[]), &first.sidecar).unwrap();
        let rules = rules(&second.document, 0);
        assert_eq!(rules.iter().filter(|rule| is_ours(rule)).count(), 1);
        assert_eq!(rules[1]["manipulators"].as_array().unwrap().len(), 5);
        assert!(second.changed && !second.edited_by_hand);
    }

    #[test]
    fn a_second_sync_with_no_change_writes_nothing() {
        let wanted = compiled(3, &[("caps_lock", "escape")]);
        let first = sync(&karabiner(), &wanted, &Sidecar::default()).unwrap();
        let second = sync(&first.document, &wanted, &first.sidecar).unwrap();
        assert!(!second.changed);
        assert_eq!(second.sidecar, first.sidecar);
    }

    #[test]
    fn a_hand_edited_block_is_overwritten_and_noticed() {
        let first = sync(&karabiner(), &compiled(2, &[]), &Sidecar::default()).unwrap();
        let mut edited = first.document.clone();
        edited["profiles"][0]["complex_modifications"]["rules"][1]["manipulators"] = json!([]);
        let second = sync(&edited, &compiled(2, &[]), &first.sidecar).unwrap();
        assert!(second.edited_by_hand);
        assert_eq!(second.document, first.document);
    }

    #[test]
    fn a_profile_switch_moves_the_block_and_our_entries() {
        let wanted = compiled(1, &[("caps_lock", "escape")]);
        let first = sync(&karabiner(), &wanted, &Sidecar::default()).unwrap();
        let mut switched = first.document.clone();
        select(&mut switched, 1);
        let second = sync(&switched, &wanted, &first.sidecar).unwrap();
        let document = &second.document;
        assert_eq!(second.profile, "Gaming");
        assert_eq!(
            rules(document, 0).len(),
            1,
            "only the user's rule stays behind"
        );
        assert_eq!(
            document["profiles"][0]["simple_modifications"],
            karabiner()["profiles"][0]["simple_modifications"]
        );
        assert_eq!(rules(document, 1).len(), 1);
        assert_eq!(
            document["profiles"][1]["simple_modifications"][0]["from"]["key_code"],
            "caps_lock"
        );
        assert_eq!(second.sidecar.profile.as_deref(), Some("Gaming"));
    }

    #[test]
    fn a_user_simple_modification_is_never_overwritten() {
        let wanted = compiled(
            0,
            &[("right_command", "left_control"), ("caps_lock", "escape")],
        );
        let synced = sync(&karabiner(), &wanted, &Sidecar::default()).unwrap();
        assert_eq!(synced.conflicts, vec!["right_command"]);
        assert_eq!(synced.sidecar.simple_modifications, vec!["caps_lock"]);
        let entries = synced.document["profiles"][0]["simple_modifications"]
            .as_array()
            .unwrap()
            .clone();
        assert_eq!(entries[0]["to"][0]["key_code"], "right_option");
        assert_eq!(entries[1]["from"]["key_code"], "caps_lock");
    }

    #[test]
    fn a_dropped_modifier_key_change_leaves_the_profile() {
        let first = sync(
            &karabiner(),
            &compiled(0, &[("caps_lock", "escape")]),
            &Sidecar::default(),
        )
        .unwrap();
        let second = sync(&first.document, &compiled(0, &[]), &first.sidecar).unwrap();
        assert_eq!(
            second.document["profiles"][0]["simple_modifications"],
            karabiner()["profiles"][0]["simple_modifications"]
        );
        assert!(second.sidecar.simple_modifications.is_empty());
    }

    #[test]
    fn a_profile_without_rules_gets_them() {
        let mut document = karabiner();
        select(&mut document, 1);
        let synced = sync(
            &document,
            &compiled(1, &[("fn", "left_control")]),
            &Sidecar::default(),
        )
        .unwrap();
        assert_eq!(rules(&synced.document, 1).len(), 1);
        assert_eq!(synced.sidecar.simple_modifications, vec!["fn"]);
    }

    #[test]
    fn missing_profiles_are_errors_that_say_what_to_do() {
        let error = sync(&json!({}), &compiled(1, &[]), &Sidecar::default()).unwrap_err();
        assert!(
            error.to_string().contains("open Karabiner-Elements"),
            "{error}"
        );
        let mut document = karabiner();
        document["profiles"][0]["selected"] = json!(false);
        let error = sync(&document, &compiled(1, &[]), &Sidecar::default()).unwrap_err();
        assert!(error.to_string().contains("no selected profile"), "{error}");
    }

    #[test]
    fn remove_takes_back_only_what_keyremap_wrote() {
        let synced = sync(
            &karabiner(),
            &compiled(2, &[("caps_lock", "escape")]),
            &Sidecar::default(),
        )
        .unwrap();
        let removed = remove(&synced.document, &synced.sidecar);
        assert!(removed.changed);
        assert_eq!(removed.document, karabiner());
        assert!(!remove(&removed.document, &Sidecar::default()).changed);
    }
}
