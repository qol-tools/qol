use serde::{Deserialize, Serialize};
use serde_json::Value;

use qol_config::config_dir;
use qol_config::profile::NOTIFICATIONS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NativeHandler {
    Qol,
    Os,
    Both,
}

pub fn stored_handler() -> NativeHandler {
    stored_config().map_or(NativeHandler::Qol, |settings| parse_config(&settings))
}

fn stored_config() -> Option<Value> {
    config_dir().map(|dir| Value::Object(NOTIFICATIONS.read(&dir)))
}

pub fn enabled_now() -> bool {
    stored_config().is_none_or(|settings| parse_enabled(&settings))
}

fn parse_enabled(settings: &Value) -> bool {
    settings
        .get(qol_conventions::NOTIFICATION_ENABLED_KEY)
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

fn parse_config(settings: &Value) -> NativeHandler {
    if let Some(handler) = settings.get("handler").and_then(Value::as_str) {
        return match handler {
            "qol" => NativeHandler::Qol,
            "os" => NativeHandler::Os,
            "both" => NativeHandler::Both,
            _ => NativeHandler::Qol,
        };
    }
    match settings
        .get("use_system_notifications")
        .and_then(Value::as_bool)
    {
        Some(true) => NativeHandler::Both,
        _ => NativeHandler::Qol,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(contents: &str) -> NativeHandler {
        parse_config(&serde_json::from_str(contents).unwrap())
    }

    #[test]
    fn parses_handler_key() {
        assert_eq!(parse(r#"{"handler": "os"}"#), NativeHandler::Os);
        assert_eq!(parse(r#"{"handler": "both"}"#), NativeHandler::Both);
        assert_eq!(parse(r#"{"handler": "qol"}"#), NativeHandler::Qol);
    }

    #[test]
    fn handler_key_wins_over_legacy_key() {
        assert_eq!(
            parse(r#"{"handler": "qol", "use_system_notifications": true}"#),
            NativeHandler::Qol
        );
    }

    #[test]
    fn parses_legacy_key() {
        assert_eq!(
            parse(r#"{"use_system_notifications": true}"#),
            NativeHandler::Both
        );
        assert_eq!(
            parse(r#"{"use_system_notifications": false}"#),
            NativeHandler::Qol
        );
    }

    #[test]
    fn defaults_to_qol_when_unset() {
        assert_eq!(parse(r#"{}"#), NativeHandler::Qol);
    }

    #[test]
    fn notifications_stay_on_unless_turned_off() {
        let cases = [
            (r#"{}"#, true),
            (r#"{"handler": "os"}"#, true),
            (r#"{"enabled": true}"#, true),
            (r#"{"enabled": false}"#, false),
            (r#"{"handler": "both", "enabled": false}"#, false),
            (r#"{"enabled": "no"}"#, true),
        ];
        for (contents, expected) in cases {
            assert_eq!(
                parse_enabled(&serde_json::from_str(contents).unwrap()),
                expected,
                "{contents}"
            );
        }
    }

    #[test]
    fn unknown_handler_defaults_to_qol() {
        assert_eq!(parse(r#"{"handler": "native"}"#), NativeHandler::Qol);
    }

    #[test]
    fn the_gate_reads_the_mute_of_this_computer_and_the_synced_handler() {
        let dir = tempfile::tempdir().unwrap();
        let settings = serde_json::json!({ "handler": "both", "enabled": false });
        NOTIFICATIONS
            .write_in(dir.path(), settings.as_object().unwrap())
            .unwrap();

        let stored = Value::Object(NOTIFICATIONS.read_in(dir.path()));
        assert_eq!(parse_config(&stored), NativeHandler::Both);
        assert!(!parse_enabled(&stored));
    }
}
