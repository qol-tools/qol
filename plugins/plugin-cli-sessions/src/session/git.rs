use std::process::Command;

use std::collections::HashMap;

const BRANCH_TTL_SECS: u64 = 30;

#[derive(Default)]
pub struct BranchCache {
    entries: HashMap<String, BranchEntry>,
}

struct BranchEntry {
    value: Option<String>,
    checked_at: u64,
}

impl BranchCache {
    pub fn branch(&mut self, cwd: &str, now: u64) -> Option<String> {
        if let Some(entry) = self.entries.get(cwd) {
            if now.saturating_sub(entry.checked_at) < BRANCH_TTL_SECS {
                return entry.value.clone();
            }
        }

        let value = branch(cwd);
        self.entries.insert(
            cwd.to_string(),
            BranchEntry {
                value: value.clone(),
                checked_at: now,
            },
        );
        value
    }
}

pub fn branch(cwd: &str) -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(cwd)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!s.is_empty() && s != "HEAD").then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_cache_reuses_fresh_entries() {
        let mut cache = BranchCache::default();
        cache.entries.insert(
            "/tmp/not-a-real-repo".to_string(),
            BranchEntry {
                value: Some("cached".to_string()),
                checked_at: 10,
            },
        );

        assert_eq!(
            cache.branch("/tmp/not-a-real-repo", 20).as_deref(),
            Some("cached")
        );
    }

    #[test]
    fn branch_cache_refreshes_expired_entries() {
        let mut cache = BranchCache::default();
        cache.entries.insert(
            "/tmp/not-a-real-repo".to_string(),
            BranchEntry {
                value: Some("cached".to_string()),
                checked_at: 1,
            },
        );

        assert_eq!(cache.branch("/tmp/not-a-real-repo", 100), None);
    }
}
