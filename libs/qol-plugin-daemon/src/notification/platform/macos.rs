use std::path::Path;
use std::process::{Command, Stdio};

use super::NotificationPlatform;

const ASSERTIONS_PATH: &str = "Library/DoNotDisturb/DB/Assertions.json";
const LEGACY_DND_DOMAIN: &str = "com.apple.notificationcenterui";
const LEGACY_DND_KEY: &str = "doNotDisturb";

pub(super) struct Platform;

impl NotificationPlatform for Platform {
    fn send_notification(&self, title: &str, message: &str) -> bool {
        send_osascript_notification(title, message)
    }

    fn os_do_not_disturb(&self) -> Option<bool> {
        let home = std::env::var("HOME").ok()?;
        let assertions = read_json(&Path::new(&home).join(ASSERTIONS_PATH))
            .and_then(|value| parse_assertions_active(&value));
        if assertions.is_some() {
            return assertions;
        }
        legacy_dnd_active()
    }

    fn acquire_inhibit(&self) -> Option<NotificationInhibit> {
        None
    }
}

pub struct NotificationInhibit;

fn read_json(path: &Path) -> Option<serde_json::Value> {
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

fn parse_assertions_active(value: &serde_json::Value) -> Option<bool> {
    let data = value.get("data")?.as_array()?;
    for entry in data {
        let records = entry
            .get("storeAssertionRecords")
            .and_then(serde_json::Value::as_array);
        if records.is_some_and(|records| !records.is_empty()) {
            return Some(true);
        }
    }
    Some(false)
}

fn legacy_dnd_active() -> Option<bool> {
    let output = Command::new("defaults")
        .args(["read", LEGACY_DND_DOMAIN, LEGACY_DND_KEY])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    parse_defaults_bool(&String::from_utf8_lossy(&output.stdout))
}

fn parse_defaults_bool(output: &str) -> Option<bool> {
    match output.trim() {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

fn send_osascript_notification(title: &str, message: &str) -> bool {
    let script = format!(
        "display notification {} with title {}",
        applescript_quote(message),
        applescript_quote(title)
    );

    Command::new("osascript")
        .arg("-e")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn applescript_quote(input: &str) -> String {
    let escaped = input.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn applescript_quote_escapes_quotes_and_backslashes() {
        assert_eq!(
            super::applescript_quote(r#"say "hi" from C:\tmp"#),
            r#""say \"hi\" from C:\\tmp""#
        );
    }

    #[test]
    fn real_assertions_shape_with_records_is_active() {
        let value = json!({
            "data": [{
                "storeInvalidationRequestRecords": [{
                    "invalidationRequestPredicate": {"invalidationPredicateType": "any"},
                    "invalidationRequestReason": "user-changed-state"
                }],
                "storeAssertionRecords": [{
                    "assertionUUID": "6E7EEB49-ABB7-4B40-A58E-4AEA0511D40B",
                    "assertionSource": {"assertionClientIdentifier": "com.apple.controlcenter.dnd"},
                    "assertionDetails": {
                        "assertionDetailsIdentifier": "com.apple.controlcenter.dnd",
                        "assertionDetailsModeIdentifier": "com.apple.donotdisturb.mode.default"
                    }
                }]
            }],
            "header": {"version": 8}
        });
        assert_eq!(parse_assertions_active(&value), Some(true));
    }

    #[test]
    fn real_assertions_shape_without_records_is_inactive() {
        let value = json!({
            "data": [{
                "storeInvalidationRequestRecords": [],
                "storeAssertionRecords": []
            }],
            "header": {"version": 8}
        });
        assert_eq!(parse_assertions_active(&value), Some(false));
    }

    #[test]
    fn entry_without_records_key_is_inactive() {
        let value = json!({"data": [{"storeInvalidationRequestRecords": []}]});
        assert_eq!(parse_assertions_active(&value), Some(false));
    }

    #[test]
    fn multiple_entries_second_with_records_is_active() {
        let value = json!({"data": [
            {"storeAssertionRecords": []},
            {"storeAssertionRecords": [{"assertionUUID": "x"}]}
        ]});
        assert_eq!(parse_assertions_active(&value), Some(true));
    }

    #[test]
    fn top_level_array_schema_is_unknown() {
        let value = json!([{"storeAssertionRecords": []}]);
        assert_eq!(parse_assertions_active(&value), None);
    }

    #[test]
    fn missing_data_key_is_unknown() {
        let value = json!({"header": {"version": 8}});
        assert_eq!(parse_assertions_active(&value), None);
    }

    #[test]
    fn parses_legacy_defaults_output() {
        assert_eq!(parse_defaults_bool("1\n"), Some(true));
        assert_eq!(parse_defaults_bool("0\n"), Some(false));
        assert_eq!(parse_defaults_bool(""), None);
        assert_eq!(parse_defaults_bool("does not exist\n"), None);
    }
}
