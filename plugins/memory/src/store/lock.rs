use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::store::Store;

pub const STALE_AFTER: Duration = Duration::from_secs(600);

#[derive(Debug)]
pub struct DistillLock {
    path: PathBuf,
}

impl DistillLock {
    pub fn acquire(store: &Store, mode: &str) -> anyhow::Result<Option<DistillLock>> {
        std::fs::create_dir_all(store.root())?;
        let path = store.distill_lock_path();
        for _ in 0..2 {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    let payload = serde_json::json!({
                        "pid": std::process::id(),
                        "started_at": crate::text::now_iso(),
                        "mode": mode
                    });
                    file.write_all(format!("{}\n", payload).as_bytes())?;
                    return Ok(Some(DistillLock { path }));
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    if started_at_is_stale(&path) {
                        let _ = std::fs::remove_file(&path);
                    } else {
                        return Ok(None);
                    }
                }
                Err(err) => return Err(err.into()),
            }
        }
        Ok(None)
    }

    pub fn acquire_wait(store: &Store, mode: &str, wait: Duration) -> anyhow::Result<DistillLock> {
        let deadline = std::time::Instant::now() + wait;
        loop {
            if let Some(lock) = Self::acquire(store, mode)? {
                return Ok(lock);
            }
            if std::time::Instant::now() >= deadline {
                anyhow::bail!("qol-memory: store is locked by another writer");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for DistillLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn started_at_is_stale(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    let Some(started_at) = value.get("started_at").and_then(|item| item.as_str()) else {
        return false;
    };
    let started_ms = crate::text::parse_iso_millis(Some(started_at));
    if started_ms == 0 {
        return false;
    }
    now_millis() - started_ms > STALE_AFTER.as_millis() as i64
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "qol-memory-lock-{}-{}-{}",
                tag,
                std::process::id(),
                nanos
            ));
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn lock_stale_after_ten_minutes_is_stolen() {
        assert_eq!(STALE_AFTER, Duration::from_secs(600));
        let dir = TempDir::new("stale");
        let store = Store::resolve(Some(dir.0.as_path())).unwrap();
        let stale_payload = "{\"started_at\":\"2020-01-01T00:00:00.000Z\"}\n";
        std::fs::write(store.distill_lock_path(), stale_payload).unwrap();
        let stolen = DistillLock::acquire(&store, "test").unwrap();
        assert!(stolen.is_some());
        drop(stolen);
        let fresh = format!(
            "{{\"pid\":1,\"started_at\":\"{}\",\"mode\":\"fresh\"}}\n",
            crate::text::now_iso()
        );
        std::fs::write(store.distill_lock_path(), fresh).unwrap();
        assert!(DistillLock::acquire(&store, "test").unwrap().is_none());
    }

    #[test]
    fn acquire_wait_times_out_with_the_contract_message() {
        let dir = TempDir::new("wait");
        let store = Store::resolve(Some(dir.0.as_path())).unwrap();
        let held = DistillLock::acquire(&store, "test").unwrap().unwrap();
        let wait = Duration::from_millis(40);
        let error = DistillLock::acquire_wait(&store, "test", wait).unwrap_err();
        assert_eq!(
            error.to_string(),
            "qol-memory: store is locked by another writer"
        );
        drop(held);
        assert!(DistillLock::acquire(&store, "test").unwrap().is_some());
    }
}
