use std::collections::VecDeque;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use anyhow::{anyhow, bail, Result};

use super::banner::{self, BannerMatch};

const CAPACITY: usize = 32;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static STORE: Mutex<Store> = Mutex::new(Store::new());

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum AppRef {
    BundleId(String),
    DisplayName(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Entry {
    pub(super) app: AppRef,
    pub(super) matcher: Option<BannerMatch>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Pressed,
    Activated,
    Missing,
}

pub(super) struct Store {
    entries: VecDeque<(String, Entry)>,
}

impl Store {
    pub(super) const fn new() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }

    pub(super) fn insert(&mut self, id: &str, entry: Entry) {
        while self.entries.len() >= CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back((id.to_string(), entry));
    }

    pub(super) fn pop(&mut self, id: &str) -> Option<Entry> {
        let position = self.entries.iter().position(|(known, _)| known == id)?;
        self.entries.remove(position).map(|(_, entry)| entry)
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }
}

pub(super) fn next_id() -> String {
    let counter = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("mb-{counter:016x}")
}

pub(super) fn insert(id: &str, entry: Entry) {
    lock().insert(id, entry);
}

fn lock() -> std::sync::MutexGuard<'static, Store> {
    STORE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(super) fn activate(id: &str) -> Result<Outcome> {
    let Some(entry) = lock().pop(id) else {
        return Ok(Outcome::Missing);
    };
    if let Some(matcher) = entry.matcher.as_ref() {
        match banner::press(matcher) {
            Ok(()) => return Ok(Outcome::Pressed),
            Err(error) => log::debug!(
                "notification mirror: AXPress failed, falling back to {app:?}: {error:#}",
                app = entry.app,
            ),
        }
    }
    activate_app(&entry.app).map(|()| Outcome::Activated)
}

fn activate_app(app: &AppRef) -> Result<()> {
    let (flag, value) = match app {
        AppRef::BundleId(id) => ("-b", id.as_str()),
        AppRef::DisplayName(name) => ("-a", name.as_str()),
    };
    if value.is_empty() {
        bail!("empty app reference");
    }
    let status = Command::new("/usr/bin/open")
        .arg(flag)
        .arg(value)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| anyhow!("failed to run open: {error}"))?;
    if !status.success() {
        bail!("open {flag} {value} exited with {status}");
    }
    Ok(())
}

pub(in super::super) fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn banner(display_name: &str, banner_id: &str) -> Entry {
        Entry {
            app: AppRef::DisplayName(display_name.to_string()),
            matcher: Some(BannerMatch {
                pid: 42,
                banner_id: banner_id.to_string(),
            }),
        }
    }

    fn db(bundle_id: &str) -> Entry {
        Entry {
            app: AppRef::BundleId(bundle_id.to_string()),
            matcher: None,
        }
    }

    #[test]
    fn is_safe_id_cases() {
        let too_long = "a".repeat(33);
        let cases = [
            ("mb-0000000000000001", true),
            ("abc_def-GH12", true),
            ("", false),
            (too_long.as_str(), false),
            ("has/slash", false),
            ("has.dot", false),
            ("has newline\n", false),
            ("has space", false),
        ];
        for (input, want) in cases {
            assert_eq!(is_safe_id(input), want, "input {input:?}");
        }
    }

    #[test]
    fn insert_and_pop_round_trip_once_per_id() {
        let mut store = Store::new();
        store.insert("mb-1", banner("Teams", "AX-1"));
        assert_eq!(store.len(), 1);
        let popped = store.pop("mb-1").expect("entry retained");
        assert_eq!(popped.app, AppRef::DisplayName("Teams".into()));
        assert_eq!(
            popped.matcher.map(|m| m.banner_id),
            Some("AX-1".to_string()),
        );
        assert_eq!(store.len(), 0);
        assert_eq!(store.pop("mb-1"), None);
    }

    #[test]
    fn insert_evicts_oldest_at_capacity() {
        let mut store = Store::new();
        let mut ids = Vec::new();
        for index in 0..CAPACITY + 5 {
            let id = format!("mb-{index:04x}");
            store.insert(&id, db(&format!("com.app.{index}")));
            ids.push(id);
        }
        assert_eq!(store.len(), CAPACITY);
        for stale in &ids[..5] {
            assert_eq!(store.pop(stale), None);
        }
        for kept in &ids[5..] {
            assert!(store.pop(kept).is_some(), "expected {kept} to be retained");
        }
    }

    #[test]
    fn pop_returns_entry_once_per_id() {
        let mut store = Store::new();
        store.insert("mb-a", db("com.a"));
        store.insert("mb-b", db("com.b"));
        assert!(store.pop("mb-a").is_some());
        assert!(store.pop("mb-a").is_none());
        assert!(store.pop("mb-b").is_some());
    }

    #[test]
    fn pop_of_unknown_id_leaves_store_intact() {
        let mut store = Store::new();
        store.insert("mb-1", db("com.a"));
        assert_eq!(store.pop("mb-missing"), None);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn next_id_is_monotonic_and_safe() {
        let mut seen = Vec::new();
        for _ in 0..8 {
            let id = next_id();
            assert!(is_safe_id(&id));
            assert!(!seen.contains(&id));
            seen.push(id);
        }
    }
}
