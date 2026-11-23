use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

use qol_config::config_dir;
use qol_config::profile::NOTIFICATIONS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NativeHandler {
    Qol,
    Os,
    Both,
}

pub fn enabled_now() -> bool {
    config_dir().is_none_or(|dir| enabled_in(&dir))
}

fn enabled_in(config_dir: &Path) -> bool {
    NOTIFICATIONS
        .read(config_dir)
        .get(qol_conventions::NOTIFICATION_ENABLED_KEY)
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_config::profile::{Scope, ACTIVE_PROFILE_FILE, PROFILE_DIR};

    #[test]
    fn the_gate_reads_this_computers_mute_in_the_active_profile() {
        let muted = Some(r#"{"enabled": false}"#);
        let cases = [
            ("work", None, None, true),
            ("work", muted, None, false),
            ("work", None, muted, true),
            ("work", Some(r#"{"enabled": true}"#), muted, true),
            ("work", Some(r#"{"enabled": "no"}"#), None, true),
            ("work", Some("not json"), None, true),
            ("default", muted, None, true),
        ];
        for (profile, device, core, expected) in cases {
            let config = tempfile::tempdir().unwrap();
            let root = config.path().join(PROFILE_DIR);
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join(ACTIVE_PROFILE_FILE), "work\n").unwrap();
            for (scope, contents) in [(Scope::Device, device), (Scope::Core, core)] {
                let Some(contents) = contents else {
                    continue;
                };
                let path = NOTIFICATIONS.path_in(&root.join(profile), scope);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, contents).unwrap();
            }

            assert_eq!(
                enabled_in(config.path()),
                expected,
                "{profile} {device:?} {core:?}"
            );
        }
    }
}
