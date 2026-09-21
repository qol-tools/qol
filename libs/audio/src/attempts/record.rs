use serde::{Deserialize, Serialize};

use crate::devices::Identity;
use crate::AudioError;

use super::lease::{self, Scope};

/// How long a choice is meant to last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifetime {
    /// Lives with the process that made it and is released when that ends.
    PortableSession,
    /// Survives its daemon and persists until an explicit release.
    ResidentPolicy,
}

/// Who currently owns the choice of default output.
///
/// A vanished Portable owner triggers release recovery, not permission to
/// overwrite unreleased state, so the epoch is part of the record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ownership {
    pub owner: String,
    pub epoch: u64,
    pub lifetime: Lifetime,
    #[serde(with = "identity_wire")]
    pub output: Identity,
}

/// What a read of the durable state can conclude.
///
/// Corrupt or unreadable state is never reported as no choice: it fails closed
/// so that automatic adoption stops and a diagnosis surfaces instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnershipState {
    None,
    Held(Ownership),
    Unreadable(String),
}

const OWNERSHIP_SCHEMA_VERSION: u32 = 1;
const OWNERSHIP_RECORD_FILE: &str = "ownership.json";
const RECORD_MODE: u32 = 0o600;

#[derive(Debug, Serialize, Deserialize)]
struct Envelope<T> {
    schema_version: u32,
    checksum: String,
    body: T,
}

pub fn read_ownership() -> Result<OwnershipState, AudioError> {
    let path = ownership_path()?;
    match read_record::<Ownership>(&path, OWNERSHIP_SCHEMA_VERSION) {
        Ok(None) => Ok(OwnershipState::None),
        Ok(Some(ownership)) => Ok(OwnershipState::Held(ownership)),
        Err(reason) => Ok(OwnershipState::Unreadable(reason)),
    }
}

/// Writes atomically under the lock the record belongs to, before the mutation
/// it describes.
pub fn write_ownership(ownership: &Ownership) -> Result<(), AudioError> {
    lease::require_held(&Scope::GlobalDefault)?;
    write_record(&ownership_path()?, OWNERSHIP_SCHEMA_VERSION, ownership)
}

pub fn clear_ownership(epoch: u64) -> Result<(), AudioError> {
    lease::require_held(&Scope::GlobalDefault)?;
    let path = ownership_path()?;
    match read_record::<Ownership>(&path, OWNERSHIP_SCHEMA_VERSION) {
        Ok(None) => Ok(()),
        Ok(Some(ownership)) if ownership.epoch == epoch => match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(AudioError::Operation(format!(
                "cannot clear the ownership record {}: {error}",
                path.display()
            ))),
        },
        Ok(Some(_)) => Ok(()),
        Err(reason) => Err(AudioError::Operation(format!(
            "refusing to clear the ownership record {}: {reason}",
            path.display()
        ))),
    }
}

pub fn abandon_ownership() -> Result<(), AudioError> {
    lease::require_held(&Scope::GlobalDefault)?;
    let path = ownership_path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AudioError::Operation(format!(
            "cannot clear the ownership record {}: {error}",
            path.display()
        ))),
    }
}

mod identity_wire {
    use serde::{Deserialize, Deserializer, Serializer};

    use crate::devices::Identity;

    pub(super) fn serialize<S: Serializer>(
        identity: &Identity,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(identity.as_str())
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Identity, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Identity::from_raw(raw))
    }
}

fn ownership_path() -> Result<std::path::PathBuf, AudioError> {
    Ok(super::state_root()?.join(OWNERSHIP_RECORD_FILE))
}

fn write_record<T: Serialize>(
    path: &std::path::Path,
    schema_version: u32,
    body: &T,
) -> Result<(), AudioError> {
    let body_bytes = serde_json::to_vec(body).map_err(|error| {
        AudioError::Operation(format!("cannot serialize the sound record: {error}"))
    })?;
    let checksum = format!("{:016x}", super::fnv1a64(&body_bytes));
    let envelope = Envelope {
        schema_version,
        checksum,
        body,
    };
    let content = serde_json::to_vec(&envelope).map_err(|error| {
        AudioError::Operation(format!("cannot serialize the sound record: {error}"))
    })?;
    qol_fs::atomic_write_durable_mode(path, &content, RECORD_MODE).map_err(|error| {
        AudioError::Operation(format!(
            "cannot write the sound record {}: {error}",
            path.display()
        ))
    })
}

fn read_record<T>(path: &std::path::Path, expected_schema: u32) -> Result<Option<T>, String>
where
    T: Serialize + serde::de::DeserializeOwned,
{
    let content = match std::fs::read(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
    };
    let envelope: Envelope<T> = serde_json::from_slice(&content)
        .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
    if envelope.schema_version != expected_schema {
        return Err(format!(
            "{} carries schema version {} (expected {expected_schema})",
            path.display(),
            envelope.schema_version
        ));
    }
    let body = serde_json::to_vec(&envelope.body)
        .map_err(|error| format!("cannot canonicalize {}: {error}", path.display()))?;
    if format!("{:016x}", super::fnv1a64(&body)) != envelope.checksum {
        return Err(format!("{} failed its checksum", path.display()));
    }
    Ok(Some(envelope.body))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::attempts::IsolatedRoot;

    fn test_output() -> Identity {
        Identity::from_raw("test-output")
    }

    fn sample_ownership(epoch: u64) -> Ownership {
        Ownership {
            owner: "plugin-sound".to_string(),
            epoch,
            lifetime: Lifetime::ResidentPolicy,
            output: test_output(),
        }
    }

    fn hold_global() -> lease::Lease {
        lease::acquire(Scope::GlobalDefault).unwrap()
    }

    #[test]
    fn ownership_round_trips() {
        let _root = IsolatedRoot::new();
        let _global = hold_global();
        assert_eq!(read_ownership().unwrap(), OwnershipState::None);
        let ownership = sample_ownership(11);
        write_ownership(&ownership).unwrap();
        assert_eq!(read_ownership().unwrap(), OwnershipState::Held(ownership));
    }

    #[test]
    fn ownership_cannot_be_written_without_the_global_lock() {
        let _root = IsolatedRoot::new();
        let error = write_ownership(&sample_ownership(1)).unwrap_err();
        assert!(matches!(error, AudioError::Operation(_)));
    }

    #[test]
    fn a_truncated_record_reads_as_unreadable_not_no_choice() {
        let _root = IsolatedRoot::new();
        let _global = hold_global();
        write_ownership(&sample_ownership(3)).unwrap();
        let path = ownership_path().unwrap();
        let content = std::fs::read(&path).unwrap();
        std::fs::write(&path, &content[..content.len() / 2]).unwrap();
        match read_ownership().unwrap() {
            OwnershipState::Unreadable(reason) => assert!(!reason.is_empty()),
            other => panic!("a truncated record must read as unreadable, got {other:?}"),
        }
    }

    #[test]
    fn a_checksum_mismatch_reads_as_unreadable() {
        let _root = IsolatedRoot::new();
        let _global = hold_global();
        write_ownership(&sample_ownership(3)).unwrap();
        let path = ownership_path().unwrap();
        let mut raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        raw["body"]["epoch"] = serde_json::json!(4);
        std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        assert!(matches!(
            read_ownership().unwrap(),
            OwnershipState::Unreadable(reason) if !reason.is_empty()
        ));
    }

    #[test]
    fn a_schema_version_mismatch_reads_as_unreadable() {
        let _root = IsolatedRoot::new();
        let _global = hold_global();
        write_ownership(&sample_ownership(3)).unwrap();
        let path = ownership_path().unwrap();
        let mut raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        raw["schema_version"] = serde_json::json!(OWNERSHIP_SCHEMA_VERSION + 1);
        std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        assert!(matches!(
            read_ownership().unwrap(),
            OwnershipState::Unreadable(_)
        ));
    }

    #[test]
    fn clear_ownership_leaves_a_newer_epoch_alone() {
        let _root = IsolatedRoot::new();
        let _global = hold_global();
        write_ownership(&sample_ownership(7)).unwrap();
        clear_ownership(6).unwrap();
        assert_eq!(
            read_ownership().unwrap(),
            OwnershipState::Held(sample_ownership(7))
        );
        clear_ownership(7).unwrap();
        assert_eq!(read_ownership().unwrap(), OwnershipState::None);
    }

    #[test]
    fn clear_ownership_refuses_an_unreadable_record() {
        let _root = IsolatedRoot::new();
        let _global = hold_global();
        write_ownership(&sample_ownership(2)).unwrap();
        let path = ownership_path().unwrap();
        let content = std::fs::read(&path).unwrap();
        std::fs::write(&path, &content[..content.len() / 2]).unwrap();
        assert!(clear_ownership(2).is_err());
        assert!(matches!(
            read_ownership().unwrap(),
            OwnershipState::Unreadable(_)
        ));
    }

    #[test]
    fn abandon_ownership_clears_an_unreadable_record() {
        let _root = IsolatedRoot::new();
        let _global = hold_global();
        write_ownership(&sample_ownership(2)).unwrap();
        let path = ownership_path().unwrap();
        let content = std::fs::read(&path).unwrap();
        std::fs::write(&path, &content[..content.len() / 2]).unwrap();
        assert!(matches!(
            read_ownership().unwrap(),
            OwnershipState::Unreadable(_)
        ));
        abandon_ownership().unwrap();
        assert_eq!(read_ownership().unwrap(), OwnershipState::None);
    }

    #[test]
    fn a_reader_never_sees_a_partial_ownership_record() {
        let _root = IsolatedRoot::new();
        let _global = hold_global();
        let stop = Arc::new(AtomicBool::new(false));
        let reader_stop = stop.clone();
        let reader = std::thread::spawn(move || {
            let mut failures = Vec::new();
            while !reader_stop.load(Ordering::Relaxed) {
                match read_ownership() {
                    Ok(OwnershipState::None) | Ok(OwnershipState::Held(_)) => {}
                    Ok(OwnershipState::Unreadable(reason)) => failures.push(reason),
                    Err(error) => failures.push(error.to_string()),
                }
            }
            failures
        });
        for epoch in 0..32 {
            let ownership = Ownership {
                owner: "x".repeat(16 * 1024),
                epoch,
                lifetime: Lifetime::PortableSession,
                output: test_output(),
            };
            write_ownership(&ownership).unwrap();
        }
        stop.store(true, Ordering::Relaxed);
        let failures = reader.join().unwrap();
        assert!(failures.is_empty(), "partial reads: {failures:?}");
    }
}
