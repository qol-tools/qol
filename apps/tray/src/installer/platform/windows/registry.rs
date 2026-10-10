use anyhow::{Context, Result};
use qol_platform::native::registry::{self, Hive, View};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::installer) enum Value {
    Text(String),
    Number(u32),
}

pub(in crate::installer) fn set(key: &str, name: &str, value: &Value) -> Result<()> {
    match value {
        Value::Text(text) => registry::write_string(Hive::CurrentUser, key, name, text),
        Value::Number(number) => registry::write_dword(Hive::CurrentUser, key, name, *number),
    }
    .with_context(|| format!("failed to write HKCU\\{key}\\{name}"))
}

pub(in crate::installer) fn text(key: &str, name: &str) -> Result<Option<String>> {
    registry::read_string(Hive::CurrentUser, key, name, View::Default)
        .with_context(|| format!("failed to read HKCU\\{key}\\{name}"))
}

pub(in crate::installer) fn delete_value(key: &str, name: &str) -> Result<()> {
    registry::delete_value(Hive::CurrentUser, key, name)
        .with_context(|| format!("failed to delete HKCU\\{key}\\{name}"))
}

pub(in crate::installer) fn delete_key(key: &str) -> Result<()> {
    registry::delete_key(Hive::CurrentUser, key)
        .with_context(|| format!("failed to delete HKCU\\{key}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: &str = r"Software\qol-tray-installer-registry-test";

    struct KeyGuard(String);

    impl Drop for KeyGuard {
        fn drop(&mut self) {
            let _ = delete_key(&self.0);
        }
    }

    #[test]
    fn values_round_trip_and_delete_cleanly() {
        let guard = KeyGuard(format!("{TEST_KEY}-{}", std::process::id()));
        let key = guard.0.as_str();
        let cases = [
            ("Name", "QoL Tray"),
            ("Path", r"C:\Users\x y\qol-tray.exe"),
            ("Empty", ""),
        ];
        for (name, value) in cases {
            set(key, name, &Value::Text(value.to_string())).unwrap();
        }
        set(key, "Flag", &Value::Number(1)).unwrap();
        for (name, value) in cases {
            assert_eq!(text(key, name).unwrap().as_deref(), Some(value), "{name}");
        }
        delete_value(key, "Name").unwrap();
        delete_value(key, "Name").unwrap();
        assert_eq!(text(key, "Name").unwrap(), None);
        delete_key(key).unwrap();
        delete_key(key).unwrap();
        assert_eq!(text(key, "Path").unwrap(), None);
    }
}
