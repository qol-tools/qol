use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

const SUPPRESS_THRESHOLD: u32 = 5;

pub(crate) struct RateLimiter {
    version: String,
    state: Mutex<HashMap<String, EntryState>>,
}

#[derive(Clone, Serialize, Deserialize)]
struct EntryState {
    count: u32,
    suppressed: bool,
    version: String,
    first_seen: String,
    last_seen: String,
    last_message: Option<String>,
    source: Option<String>,
    location: Option<String>,
}

pub(crate) enum CheckResult {
    Allowed { count: u32 },
    Suppressed { count: u32 },
    Rejected,
}

impl CheckResult {
    pub(crate) fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed { .. })
    }

    pub(crate) fn is_suppressed(&self) -> bool {
        matches!(self, Self::Suppressed { .. })
    }

    pub(crate) fn is_rejected(&self) -> bool {
        matches!(self, Self::Rejected)
    }

    pub(crate) fn count(&self) -> u32 {
        match self {
            Self::Allowed { count } | Self::Suppressed { count } => *count,
            Self::Rejected => SUPPRESS_THRESHOLD,
        }
    }
}

impl RateLimiter {
    pub(crate) fn new(version: String) -> Self {
        Self {
            version,
            state: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn load(path: &Path, version: String) -> Self {
        let mut state: HashMap<String, EntryState> = std::fs::read_to_string(path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default();

        state.retain(|_, entry| entry.version == version);

        Self {
            version,
            state: Mutex::new(state),
        }
    }

    pub(crate) fn check(&self, key: &str) -> CheckResult {
        let mut state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return CheckResult::Allowed { count: 1 },
        };

        let entry = state.entry(key.to_string()).or_insert_with(|| EntryState {
            count: 0,
            suppressed: false,
            version: self.version.clone(),
            first_seen: now_iso(),
            last_seen: now_iso(),
            last_message: None,
            source: None,
            location: None,
        });

        if entry.suppressed {
            entry.count += 1;
            entry.last_seen = now_iso();
            return CheckResult::Rejected;
        }

        entry.count += 1;
        entry.last_seen = now_iso();

        if entry.count >= SUPPRESS_THRESHOLD {
            entry.suppressed = true;
            return CheckResult::Suppressed { count: entry.count };
        }

        CheckResult::Allowed { count: entry.count }
    }

    pub(crate) fn update_entry_context(
        &self,
        key: &str,
        message: &str,
        source: &str,
        location: &str,
    ) {
        let mut state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        if let Some(entry) = state.get_mut(key) {
            entry.last_message = Some(message.to_string());
            entry.source = Some(source.to_string());
            entry.location = Some(location.to_string());
        }
    }

    pub(crate) fn save(&self, path: &Path) {
        let state = match self.state.lock() {
            Ok(s) => s,
            Err(_) => return,
        };
        let suppressed: HashMap<_, _> = state
            .iter()
            .filter(|(_, e)| e.suppressed)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if suppressed.is_empty() {
            let _ = std::fs::remove_file(path);
            return;
        }
        let Ok(content) = serde_json::to_string_pretty(&suppressed) else {
            return;
        };
        let _ = std::fs::write(path, content);
    }
}

fn now_iso() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter(version: &str) -> RateLimiter {
        RateLimiter::new(version.to_string())
    }

    #[test]
    fn first_occurrence_is_allowed() {
        let rl = limiter("1.0.0");
        assert!(rl.check("err.key").is_allowed());
    }

    #[test]
    fn occurrences_up_to_threshold_are_allowed() {
        let rl = limiter("1.0.0");
        for _ in 0..4 {
            assert!(rl.check("err.key").is_allowed());
        }
    }

    #[test]
    fn fifth_occurrence_returns_suppressed() {
        let rl = limiter("1.0.0");
        for _ in 0..4 {
            rl.check("err.key");
        }
        let result = rl.check("err.key");
        assert!(result.is_suppressed(), "5th should suppress");
    }

    #[test]
    fn after_suppression_is_rejected() {
        let rl = limiter("1.0.0");
        for _ in 0..6 {
            rl.check("err.key");
        }
        let result = rl.check("err.key");
        assert!(result.is_rejected(), "6th+ should reject");
    }

    #[test]
    fn different_keys_are_independent() {
        let rl = limiter("1.0.0");
        for _ in 0..5 {
            rl.check("key.a");
        }
        assert!(rl.check("key.b").is_allowed());
    }

    #[test]
    fn check_returns_occurrence_count() {
        let rl = limiter("1.0.0");
        let cases = [(1, 1), (2, 2), (3, 3), (4, 4), (5, 5), (6, 5)];
        for (call, expected_count) in cases {
            let result = rl.check("err.key");
            assert_eq!(
                result.count(),
                expected_count,
                "call {} should have count {}",
                call,
                expected_count
            );
        }
    }

    #[test]
    fn load_and_save_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("suppressed.json");

        let rl = limiter("1.0.0");
        for _ in 0..5 {
            rl.check("err.key");
        }
        rl.save(&path);

        let rl2 = RateLimiter::load(&path, "1.0.0".to_string());
        assert!(rl2.check("err.key").is_rejected());
    }

    #[test]
    fn version_change_resets_suppression() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("suppressed.json");

        let rl = limiter("1.0.0");
        for _ in 0..5 {
            rl.check("err.key");
        }
        rl.save(&path);

        let rl2 = RateLimiter::load(&path, "2.0.0".to_string());
        assert!(
            rl2.check("err.key").is_allowed(),
            "new version should reset"
        );
    }
}
