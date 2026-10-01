use super::dconf::{
    parse_string_array, serialize_string_array, BindingEntry, BindingReach, MatchPolicy,
};
use serde_json::{Map, Value};

pub(crate) const ROOT: &str = "/cinnamon/spices/";

const ENTRY_SEPARATOR: &str = "::";

pub(crate) struct SpiceConfig {
    pub uuid: String,
    pub config_id: String,
    pub json: String,
}

pub(crate) fn dir(uuid: &str, config_id: &str) -> String {
    format!("{ROOT}{uuid}/{config_id}/")
}

pub(crate) fn split_full_key(full_key: &str) -> Option<(&str, &str, &str)> {
    let mut parts = full_key.strip_prefix(ROOT)?.split('/');
    let (uuid, config_id, key) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || [uuid, config_id, key].iter().any(|part| part.is_empty()) {
        return None;
    }
    Some((uuid, config_id, key))
}

pub(crate) fn setting_key(full_key: &str) -> Option<&str> {
    split_full_key(full_key).map(|(_, _, key)| key)
}

pub(crate) fn parse_settings(uuid: &str, config_id: &str, json: &str) -> Vec<BindingEntry> {
    let Ok(Value::Object(settings)) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    settings
        .iter()
        .filter_map(|(key, setting)| {
            Some(BindingEntry {
                dir: dir(uuid, config_id),
                key: key.clone(),
                values: split_entries(keybinding(setting)?.get("value")?.as_str()?),
                reach: BindingReach::Managed,
                match_policy: MatchPolicy::Subset,
            })
        })
        .collect()
}

pub(crate) fn read_binding(json: &str, key: &str) -> Option<String> {
    let settings: Map<String, Value> = serde_json::from_str(json).ok()?;
    let value = keybinding(settings.get(key)?)?.get("value")?.as_str()?;
    Some(serialize_string_array(&split_entries(value)))
}

pub(crate) fn write_binding(json: &str, key: &str, array: &str) -> Option<String> {
    let entries = parse_string_array(array)?;
    update_value(json, key, |_| Some(entries.join(ENTRY_SEPARATOR)))
}

pub(crate) fn reset_binding(json: &str, key: &str) -> Option<String> {
    update_value(json, key, |setting| {
        Some(setting.get("default")?.as_str()?.to_string())
    })
}

fn update_value(
    json: &str,
    key: &str,
    next: impl FnOnce(&Map<String, Value>) -> Option<String>,
) -> Option<String> {
    let mut settings: Map<String, Value> = serde_json::from_str(json).ok()?;
    let setting = settings.get_mut(key)?.as_object_mut()?;
    keybinding(&Value::Object(setting.clone()))?;
    let value = next(setting)?;
    setting.insert("value".to_string(), Value::String(value));
    to_cinnamon_json(&settings)
}

fn keybinding(setting: &Value) -> Option<&Map<String, Value>> {
    let setting = setting.as_object()?;
    (setting.get("type")?.as_str()? == "keybinding").then_some(setting)
}

fn split_entries(value: &str) -> Vec<String> {
    value
        .split(ENTRY_SEPARATOR)
        .filter(|entry| !entry.is_empty())
        .map(String::from)
        .collect()
}

fn to_cinnamon_json(settings: &Map<String, Value>) -> Option<String> {
    use serde::Serialize;
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    settings.serialize(&mut serializer).ok()?;
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTIFICATIONS: &str = r#"{
    "keyOpen": {
        "type": "keybinding",
        "description": "Show notifications",
        "default": "<Super>n",
        "value": "<Super>n::<Primary><Alt>n"
    },
    "showEmptyTray": {
        "type": "switch",
        "default": false,
        "value": "<Super>n"
    },
    "keyClear": {
        "type": "keybinding",
        "default": "<Shift><Super>c",
        "value": ""
    }
}"#;

    #[test]
    fn only_keybinding_settings_are_scanned_with_every_entry_split_out() {
        let entries = parse_settings(
            "notifications@cinnamon.org",
            "notifications@cinnamon.org",
            NOTIFICATIONS,
        );
        let seen: Vec<(&str, &str, Vec<&str>)> = entries
            .iter()
            .map(|e| {
                (
                    e.dir.as_str(),
                    e.key.as_str(),
                    e.values.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(
            seen,
            vec![
                (
                    "/cinnamon/spices/notifications@cinnamon.org/notifications@cinnamon.org/",
                    "keyOpen",
                    vec!["<Super>n", "<Primary><Alt>n"]
                ),
                (
                    "/cinnamon/spices/notifications@cinnamon.org/notifications@cinnamon.org/",
                    "keyClear",
                    vec![]
                ),
            ]
        );
        assert!(entries
            .iter()
            .all(|e| e.match_policy == MatchPolicy::Subset));
        assert!(parse_settings("a", "a", "not json").is_empty());
    }

    #[test]
    fn a_full_key_round_trips_through_its_dir() {
        let full_key = format!("{}keyOpen", dir("notifications@cinnamon.org", "5"));
        assert_eq!(
            split_full_key(&full_key),
            Some(("notifications@cinnamon.org", "5", "keyOpen"))
        );
        assert_eq!(setting_key(&full_key), Some("keyOpen"));
        for rejected in [
            "/org/cinnamon/desktop/keybindings/wm/close",
            "/cinnamon/spices/uuid/keyOpen",
            "/cinnamon/spices/uuid/5/keyOpen/extra",
            "/cinnamon/spices/uuid//keyOpen",
        ] {
            assert_eq!(split_full_key(rejected), None, "{rejected}");
        }
    }

    #[test]
    fn writing_then_reading_a_binding_uses_the_takeover_array_form() {
        assert_eq!(
            read_binding(NOTIFICATIONS, "keyOpen").as_deref(),
            Some("['<Super>n', '<Primary><Alt>n']")
        );
        let cleared =
            write_binding(NOTIFICATIONS, "keyOpen", "['<Primary><Alt>n']").expect("write");
        assert_eq!(
            read_binding(&cleared, "keyOpen").as_deref(),
            Some("['<Primary><Alt>n']")
        );
        let emptied = write_binding(&cleared, "keyOpen", "@as []").expect("write");
        assert_eq!(read_binding(&emptied, "keyOpen").as_deref(), Some("@as []"));
        assert!(emptied.contains(r#""value": """#), "{emptied}");
        let reset = reset_binding(&emptied, "keyOpen").expect("reset");
        assert_eq!(
            read_binding(&reset, "keyOpen").as_deref(),
            Some("['<Super>n']")
        );
    }

    #[test]
    fn a_write_keeps_every_other_setting_and_refuses_non_keybindings() {
        let written = write_binding(NOTIFICATIONS, "keyOpen", "@as []").expect("write");
        let before: Value = serde_json::from_str(NOTIFICATIONS).expect("json");
        let after: Value = serde_json::from_str(&written).expect("json");
        assert_eq!(before["showEmptyTray"], after["showEmptyTray"]);
        assert_eq!(before["keyClear"], after["keyClear"]);
        assert_eq!(before["keyOpen"]["default"], after["keyOpen"]["default"]);
        assert_eq!(
            write_binding(NOTIFICATIONS, "showEmptyTray", "@as []"),
            None
        );
        assert_eq!(read_binding(NOTIFICATIONS, "showEmptyTray"), None);
        assert_eq!(write_binding(NOTIFICATIONS, "missing", "@as []"), None);
    }
}
