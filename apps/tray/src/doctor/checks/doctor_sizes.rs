use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) enum StoredSize {
    Missing,
    Bytes(u64),
    Unreadable(String),
}

#[derive(Default, Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct DoctorSizes {
    #[serde(default)]
    pub scanned_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<StoredSize>,
    #[serde(default)]
    pub prunable: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<StoredSize>,
}

impl DoctorSizes {
    pub(super) fn fresh(&self, now_ms: u64, ttl: Duration) -> bool {
        self.scanned_at_ms != 0
            && now_ms.saturating_sub(self.scanned_at_ms) <= ttl.as_millis() as u64
    }
}

pub(super) fn path_for(root: &Path) -> PathBuf {
    root.join("target")
        .join("qol-dev")
        .join("doctor-sizes.json")
}

pub(super) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

pub(super) fn load(path: &Path) -> Option<DoctorSizes> {
    let serialized = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&serialized).ok()
}

pub(super) fn save(path: &Path, sizes: &DoctorSizes) {
    let Ok(serialized) = serde_json::to_string(sizes) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, serialized);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DoctorSizes {
        DoctorSizes {
            scanned_at_ms: 1_000,
            total: Some(StoredSize::Bytes(12345)),
            prunable: 678,
            cache: Some(StoredSize::Unreadable("perm denied".to_string())),
        }
    }

    #[test]
    fn round_trip_preserves_sizes() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("doctor-sizes.json");
        save(&path, &sample());
        assert_eq!(load(&path), Some(sample()));
    }

    #[test]
    fn freshness_uses_scanned_at_within_ttl() {
        let sizes = sample();
        let ttl = Duration::from_secs(30 * 60);
        assert!(sizes.fresh(1_000, ttl));
        assert!(sizes.fresh(1_000 + ttl.as_millis() as u64, ttl));
        assert!(!sizes.fresh(1_001 + ttl.as_millis() as u64, ttl));
        assert!(!DoctorSizes::default().fresh(1_000, ttl));
    }
}
