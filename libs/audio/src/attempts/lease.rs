use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::AudioError;

use super::state_root;

/// The single machine-wide lock over the default output.
///
/// The default is one shared resource, so every mutation of it is serialized
/// on this one lock rather than on a per-output lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    GlobalDefault,
}

/// A held advisory file lock. The kernel drops it when the process dies, so a
/// crash frees it at once with no expiry to wait out, and the owner written
/// inside the file is for the message rather than for the exclusion.
#[derive(Debug)]
pub struct Lease {
    scope: Scope,
    file: File,
}

impl Lease {
    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    fn write_holder(&mut self, path: &Path) -> Result<(), AudioError> {
        let line = holder_line();
        self.file
            .set_len(0)
            .map_err(|error| lock_write_error(path, error))?;
        self.file
            .rewind()
            .map_err(|error| lock_write_error(path, error))?;
        self.file
            .write_all(line.as_bytes())
            .map_err(|error| lock_write_error(path, error))?;
        self.file
            .flush()
            .map_err(|error| lock_write_error(path, error))
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        unregister(&self.scope);
        let _ = self.file.unlock();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub owner: String,
    pub pid: u32,
    pub since_unix_seconds: u64,
}

const LOCK_DIRECTORY: &str = "locks";
const GLOBAL_LOCK_FILE: &str = "default.lock";
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

static HELD: Mutex<Vec<Scope>> = Mutex::new(Vec::new());

/// Where a lock lives. Every caller computes this the same way, hosted or
/// standalone, and it is never the runtime directory: qol-tray recreates that
/// at every start and would unlink a lock a terminal repair still holds.
pub fn lock_path(_scope: &Scope) -> Result<PathBuf, AudioError> {
    Ok(state_root()?.join(LOCK_DIRECTORY).join(GLOBAL_LOCK_FILE))
}

/// Blocks until the lock is held or the deadline passes.
pub fn acquire(scope: Scope) -> Result<Lease, AudioError> {
    acquire_within(scope, ACQUIRE_TIMEOUT)
}

pub fn acquire_within(scope: Scope, timeout: Duration) -> Result<Lease, AudioError> {
    let path = lock_path(&scope)?;
    let file = open_lock_file(&path)?;
    let deadline = Instant::now().checked_add(timeout);
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) => {
                if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                    return Err(AudioError::Operation(format!(
                        "timed out after {timeout:?} waiting for the sound lock {}",
                        path.display()
                    )));
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(TryLockError::Error(error)) => {
                return Err(AudioError::Operation(format!(
                    "cannot take the sound lock {}: {error}",
                    path.display()
                )));
            }
        }
    }
    let mut lease = Lease { scope, file };
    lease.write_holder(&path)?;
    register(&lease.scope);
    Ok(lease)
}

/// Returns `Ok(None)` when another live caller holds the lock, so the caller
/// can report who is busy instead of waiting behind it.
pub fn try_acquire(scope: Scope) -> Result<Option<Lease>, AudioError> {
    let path = lock_path(&scope)?;
    let file = open_lock_file(&path)?;
    match file.try_lock() {
        Ok(()) => {
            let mut lease = Lease { scope, file };
            lease.write_holder(&path)?;
            register(&lease.scope);
            Ok(Some(lease))
        }
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => Err(AudioError::Operation(format!(
            "cannot take the sound lock {}: {error}",
            path.display()
        ))),
    }
}

pub fn holder(scope: &Scope) -> Result<Option<Holder>, AudioError> {
    let path = lock_path(scope)?;
    let file = match OpenOptions::new().read(true).open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(AudioError::Operation(format!(
                "cannot open the sound lock {}: {error}",
                path.display()
            )))
        }
    };
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            Ok(None)
        }
        Err(TryLockError::WouldBlock) => {
            let content = std::fs::read_to_string(&path).map_err(|error| {
                AudioError::Operation(format!(
                    "cannot read the sound lock {}: {error}",
                    path.display()
                ))
            })?;
            Ok(Some(parse_holder(&content).unwrap_or_else(|| Holder {
                owner: "unknown".to_string(),
                pid: 0,
                since_unix_seconds: 0,
            })))
        }
        Err(TryLockError::Error(error)) => Err(AudioError::Operation(format!(
            "cannot inspect the sound lock {}: {error}",
            path.display()
        ))),
    }
}

pub(crate) fn require_held(scope: &Scope) -> Result<(), AudioError> {
    if is_held(scope) {
        return Ok(());
    }
    Err(AudioError::Operation(
        "the global default sound lock must be held for this mutation".to_string(),
    ))
}

fn open_lock_file(path: &Path) -> Result<File, AudioError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            AudioError::Operation(format!(
                "cannot create the sound lock directory {}: {error}",
                parent.display()
            ))
        })?;
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| {
            AudioError::Operation(format!(
                "cannot open the sound lock {}: {error}",
                path.display()
            ))
        })
}

fn lock_write_error(path: &Path, error: std::io::Error) -> AudioError {
    AudioError::Operation(format!(
        "cannot write the sound lock {}: {error}",
        path.display()
    ))
}

fn is_held(scope: &Scope) -> bool {
    held().contains(scope)
}

fn register(scope: &Scope) {
    let mut held = held();
    if !held.contains(scope) {
        held.push(scope.clone());
    }
}

fn unregister(scope: &Scope) {
    held().retain(|held_scope| held_scope != scope);
}

fn held() -> MutexGuard<'static, Vec<Scope>> {
    HELD.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn holder_line() -> String {
    let owner = std::env::var(qol_conventions::ENV_PLUGIN_ID)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(process_name);
    let pid = std::process::id();
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    format!("owner={}\npid={pid}\nsince={since}\n", sanitize(&owner))
}

fn process_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '\n' | '\r' | '=' => '_',
            other => other,
        })
        .collect()
}

fn parse_holder(content: &str) -> Option<Holder> {
    let mut owner = None;
    let mut pid = None;
    let mut since = None;
    for line in content.lines() {
        let (key, value) = line.split_once('=')?;
        match key {
            "owner" => owner = Some(value.to_string()),
            "pid" => pid = value.parse().ok(),
            "since" => since = value.parse().ok(),
            _ => {}
        }
    }
    Some(Holder {
        owner: owner?,
        pid: pid?,
        since_unix_seconds: since?,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::attempts::{IsolatedRoot, TEST_ROOT_ENV};

    const CHILD_MODE_ENV: &str = "QOL_AUDIO_LEASE_CHILD_MODE";
    const CHILD_EXPECT_ENV: &str = "QOL_AUDIO_LEASE_CHILD_EXPECT";
    const CHILD_RAN_SENTINEL: &str = "QOL_AUDIO_LEASE_CHILD_RAN";
    const CHILD_ACQUIRED_SENTINEL: &str = "QOL_AUDIO_LEASE_CHILD_ACQUIRED";
    const CHILD_BUSY_SENTINEL: &str = "QOL_AUDIO_LEASE_CHILD_BUSY";

    #[test]
    fn two_processes_cannot_hold_the_same_lock() {
        let root = IsolatedRoot::new();
        let scope = Scope::GlobalDefault;
        let held = acquire(scope.clone()).unwrap();
        assert!(try_acquire(scope.clone()).unwrap().is_none());
        assert!(child("try", Some("busy"), root.path()).success());

        drop(held);
        assert!(child("try", Some("free"), root.path()).success());
        assert!(try_acquire(scope).unwrap().is_some());
    }

    #[test]
    fn a_lock_file_still_exists_after_the_lease_drops() {
        let _root = IsolatedRoot::new();
        let path = lock_path(&Scope::GlobalDefault).unwrap();
        assert!(!path.exists());
        {
            let _lease = acquire(Scope::GlobalDefault).unwrap();
        }
        assert!(path.is_file());
    }

    #[test]
    fn a_released_lock_reports_no_holder() {
        let _root = IsolatedRoot::new();
        {
            let _lease = acquire(Scope::GlobalDefault).unwrap();
        }
        assert_eq!(holder(&Scope::GlobalDefault).unwrap(), None);
    }

    #[test]
    fn a_busy_lock_with_a_corrupt_line_is_not_free() {
        let _root = IsolatedRoot::new();
        let path = lock_path(&Scope::GlobalDefault).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not a holder line\n").unwrap();
        let busy = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        busy.try_lock().unwrap();
        let info = holder(&Scope::GlobalDefault)
            .unwrap()
            .expect("a busy lock is never free");
        assert_eq!(info.owner, "unknown");
        assert_eq!(info.pid, 0);
        assert_eq!(info.since_unix_seconds, 0);
    }

    #[test]
    fn a_dead_lock_holder_frees_its_lock() {
        let root = IsolatedRoot::new();
        let scope = Scope::GlobalDefault;
        assert!(child("hold", None, root.path()).success());
        let lease = try_acquire(scope)
            .unwrap()
            .expect("the lock is free after its holder died");
        drop(lease);
    }

    #[test]
    fn the_lock_file_names_its_holder() {
        let _root = IsolatedRoot::new();
        let lease = acquire(Scope::GlobalDefault).unwrap();
        let info = holder(lease.scope()).unwrap().expect("holder info");
        assert_eq!(info.pid, std::process::id());
        assert!(!info.owner.is_empty());
        assert!(info.since_unix_seconds > 0);
    }

    #[test]
    fn acquire_gives_up_at_its_deadline() {
        let _root = IsolatedRoot::new();
        let _held = acquire(Scope::GlobalDefault).unwrap();
        let started = Instant::now();
        let error = acquire_within(Scope::GlobalDefault, Duration::from_millis(120)).unwrap_err();
        assert!(matches!(error, AudioError::Operation(_)));
        assert!(started.elapsed() >= Duration::from_millis(120));
    }

    fn child(mode: &str, expect: Option<&str>, root: &Path) -> std::process::ExitStatus {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .arg("--exact")
            .arg("--nocapture")
            .arg("attempts::lease::tests::child_probe_lock")
            .env(CHILD_MODE_ENV, mode)
            .env(TEST_ROOT_ENV, root);
        if let Some(expect) = expect {
            command.env(CHILD_EXPECT_ENV, expect);
        }
        let output = command.output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stdout.contains(CHILD_RAN_SENTINEL),
            "the child probe never ran: {stdout:?} {stderr:?}"
        );
        let expected = match (mode, expect) {
            ("hold", _) | ("try", Some("free")) => CHILD_ACQUIRED_SENTINEL,
            ("try", Some("busy")) => CHILD_BUSY_SENTINEL,
            _ => panic!("unknown child mode {mode}"),
        };
        let unexpected = if expected == CHILD_ACQUIRED_SENTINEL {
            CHILD_BUSY_SENTINEL
        } else {
            CHILD_ACQUIRED_SENTINEL
        };
        assert!(
            stdout.contains(expected),
            "the child probe did not report {expected}: {stdout:?} {stderr:?}"
        );
        assert!(
            !stdout.contains(unexpected),
            "the child probe also reported {unexpected}: {stdout:?} {stderr:?}"
        );
        output.status
    }

    #[test]
    fn child_probe_lock() {
        let Ok(mode) = std::env::var(CHILD_MODE_ENV) else {
            return;
        };
        println!("{CHILD_RAN_SENTINEL}");
        match mode.as_str() {
            "try" => {
                let acquired = try_acquire(Scope::GlobalDefault).unwrap().is_some();
                let verdict = if acquired {
                    CHILD_ACQUIRED_SENTINEL
                } else {
                    CHILD_BUSY_SENTINEL
                };
                println!("{verdict}");
                let expected = std::env::var(CHILD_EXPECT_ENV).as_deref() == Ok("free");
                std::process::exit(if acquired == expected { 0 } else { 1 });
            }
            "hold" => {
                let _lease = acquire(Scope::GlobalDefault).expect("the child takes the lock");
                println!("{CHILD_ACQUIRED_SENTINEL}");
                std::process::exit(0);
            }
            _ => std::process::exit(2),
        }
    }
}
