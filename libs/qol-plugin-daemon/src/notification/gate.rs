use serde::{Deserialize, Serialize};

use qol_config::config_dir;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NativeHandler {
    Qol,
    Os,
    Both,
}

pub fn stored_handler() -> NativeHandler {
    let Some(path) = config_dir().map(|dir| dir.join("notifications.json")) else {
        return NativeHandler::Qol;
    };
    let Ok(contents) = std::fs::read_to_string(path) else {
        return NativeHandler::Qol;
    };
    parse_config(&contents)
}

pub fn native_allowed(handler: NativeHandler) -> bool {
    !matches!(handler, NativeHandler::Qol)
}

pub fn native_allowed_now() -> bool {
    native_allowed(stored_handler())
}

fn parse_config(contents: &str) -> NativeHandler {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(contents) else {
        return NativeHandler::Qol;
    };
    if let Some(handler) = value.get("handler").and_then(serde_json::Value::as_str) {
        return match handler {
            "qol" => NativeHandler::Qol,
            "os" => NativeHandler::Os,
            "both" => NativeHandler::Both,
            _ => NativeHandler::Qol,
        };
    }
    match value
        .get("use_system_notifications")
        .and_then(serde_json::Value::as_bool)
    {
        Some(true) => NativeHandler::Both,
        _ => NativeHandler::Qol,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn write_temp_config(contents: &str) -> PathBuf {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("qol-notification-gate-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notifications.json");
        fs::write(&path, contents).unwrap();
        path
    }

    fn parse_file(path: &PathBuf) -> NativeHandler {
        parse_config(&fs::read_to_string(path).unwrap())
    }

    #[test]
    fn qol_handler_blocks_native() {
        assert!(!native_allowed(NativeHandler::Qol));
    }

    #[test]
    fn os_and_both_handlers_allow_native() {
        assert!(native_allowed(NativeHandler::Os));
        assert!(native_allowed(NativeHandler::Both));
    }

    #[test]
    fn parses_handler_key() {
        let path = write_temp_config(r#"{"handler": "os"}"#);
        assert_eq!(parse_file(&path), NativeHandler::Os);
        let path = write_temp_config(r#"{"handler": "both"}"#);
        assert_eq!(parse_file(&path), NativeHandler::Both);
        let path = write_temp_config(r#"{"handler": "qol"}"#);
        assert_eq!(parse_file(&path), NativeHandler::Qol);
    }

    #[test]
    fn handler_key_wins_over_legacy_key() {
        let path = write_temp_config(r#"{"handler": "qol", "use_system_notifications": true}"#);
        assert_eq!(parse_file(&path), NativeHandler::Qol);
    }

    #[test]
    fn parses_legacy_key() {
        let path = write_temp_config(r#"{"use_system_notifications": true}"#);
        assert_eq!(parse_file(&path), NativeHandler::Both);
        let path = write_temp_config(r#"{"use_system_notifications": false}"#);
        assert_eq!(parse_file(&path), NativeHandler::Qol);
    }

    #[test]
    fn defaults_to_qol_when_unset() {
        let path = write_temp_config(r#"{}"#);
        assert_eq!(parse_file(&path), NativeHandler::Qol);
    }

    #[test]
    fn corrupt_json_defaults_to_qol() {
        let path = write_temp_config(r#"{not json"#);
        assert_eq!(parse_file(&path), NativeHandler::Qol);
    }

    #[test]
    fn unknown_handler_defaults_to_qol() {
        let path = write_temp_config(r#"{"handler": "native"}"#);
        assert_eq!(parse_file(&path), NativeHandler::Qol);
    }
}
