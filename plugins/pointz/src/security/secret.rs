use anyhow::{Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

const SECRET_FILE: &str = "pairing-secret";
const QUARANTINE_EXTENSION: &str = "unreadable";

pub struct ServerIdentity([u8; 32]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExistingSecretState {
    Missing,
    Present,
    Invalid,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExistingSecretInspection {
    pub path: Option<PathBuf>,
    pub state: ExistingSecretState,
    pub file_type: &'static str,
    pub bytes: Option<u64>,
    pub readonly: Option<bool>,
    pub issue: Option<String>,
}

impl ServerIdentity {
    pub fn load_or_create() -> Result<Self> {
        let path = secret_path().context("PointZ data directory is unavailable")?;
        Self::load_or_create_at(&path)
    }

    fn load_or_create_at(path: &Path) -> Result<Self> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata)
                if metadata.file_type().is_file() && !metadata.file_type().is_symlink() =>
            {
                match Self::from_stored_bytes(&std::fs::read(path)?) {
                    Ok(identity) => return Ok(identity),
                    Err(error) => quarantine(path, &error)?,
                }
            }
            Ok(_) => anyhow::bail!("PointZ identity seed path is not a regular file"),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Self::generate_at(path)
    }

    fn generate_at(path: &Path) -> Result<Self> {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes)
            .map_err(|error| anyhow::anyhow!("failed to generate PointZ identity seed: {error}"))?;
        let identity = Self(bytes);
        qol_fs::atomic_write_private(path, identity.encode_seed().as_bytes()).with_context(
            || {
                format!(
                    "failed to persist PointZ identity seed at {}",
                    path.display()
                )
            },
        )?;
        Ok(identity)
    }

    fn from_stored_bytes(stored: &[u8]) -> Result<Self> {
        let text =
            std::str::from_utf8(stored).context("PointZ identity seed is not valid UTF-8")?;
        Self::decode(text.trim())
    }

    fn encode_seed(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }

    pub fn server_id(&self) -> String {
        let digest = Sha256::digest(self.0);
        URL_SAFE_NO_PAD.encode(&digest[..12])
    }

    fn decode(encoded: &str) -> Result<Self> {
        let decoded = URL_SAFE_NO_PAD
            .decode(encoded)
            .context("PointZ identity seed is not valid base64url")?;
        let bytes: [u8; 32] = decoded
            .try_into()
            .map_err(|_| anyhow::anyhow!("PointZ identity seed must be 32 bytes"))?;
        Ok(Self(bytes))
    }

    #[cfg(test)]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub(crate) fn inspect_existing() -> ExistingSecretInspection {
        let Some(path) = secret_path() else {
            return ExistingSecretInspection {
                path: None,
                state: ExistingSecretState::Unavailable,
                file_type: "unavailable",
                bytes: None,
                readonly: None,
                issue: Some("PointZ data directory is unavailable".to_string()),
            };
        };
        inspect_path(&path)
    }
}

fn quarantine(path: &Path, reason: &anyhow::Error) -> Result<()> {
    let quarantined = path.with_extension(QUARANTINE_EXTENSION);
    std::fs::rename(path, &quarantined).with_context(|| {
        format!(
            "failed to set aside the unreadable PointZ identity seed at {}",
            path.display()
        )
    })?;
    log::warn!(
        "Replacing the unreadable PointZ identity seed ({reason}); the old file is at {}",
        quarantined.display()
    );
    Ok(())
}

fn inspect_path(path: &Path) -> ExistingSecretInspection {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return missing_inspection(path);
        }
        Err(error) => {
            return metadata_error_inspection(path, error);
        }
    };
    inspect_metadata(path, metadata)
}

fn inspect_metadata(path: &Path, metadata: std::fs::Metadata) -> ExistingSecretInspection {
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return non_regular_inspection(path, "symlink", metadata.len());
    }
    if !file_type.is_file() {
        return non_regular_inspection(path, "other", metadata.len());
    }
    inspect_regular_file(path, metadata)
}

fn inspect_regular_file(path: &Path, metadata: std::fs::Metadata) -> ExistingSecretInspection {
    ExistingSecretInspection {
        path: Some(path.to_path_buf()),
        state: ExistingSecretState::Present,
        file_type: "regular",
        bytes: Some(metadata.len()),
        readonly: Some(metadata.permissions().readonly()),
        issue: None,
    }
}

fn missing_inspection(path: &Path) -> ExistingSecretInspection {
    ExistingSecretInspection {
        path: Some(path.to_path_buf()),
        state: ExistingSecretState::Missing,
        file_type: "missing",
        bytes: None,
        readonly: None,
        issue: None,
    }
}

fn metadata_error_inspection(path: &Path, error: std::io::Error) -> ExistingSecretInspection {
    ExistingSecretInspection {
        path: Some(path.to_path_buf()),
        state: ExistingSecretState::Invalid,
        file_type: "unknown",
        bytes: None,
        readonly: None,
        issue: Some(error.to_string()),
    }
}

fn non_regular_inspection(
    path: &Path,
    file_type: &'static str,
    bytes: u64,
) -> ExistingSecretInspection {
    ExistingSecretInspection {
        path: Some(path.to_path_buf()),
        state: ExistingSecretState::Invalid,
        file_type,
        bytes: Some(bytes),
        readonly: None,
        issue: Some("PointZ identity seed path is not a regular file".to_string()),
    }
}

fn secret_path() -> Option<PathBuf> {
    let plugin_id = qol_config::plugin_id_from_env(env!("QOL_PLUGIN_ID"));
    Some(
        qol_config::data_dir()?
            .join("plugins")
            .join(plugin_id)
            .join(SECRET_FILE),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn encoded_seed_round_trips_and_has_a_stable_server_id() {
        let identity = ServerIdentity::from_bytes([3; 32]);

        let decoded = ServerIdentity::decode(&identity.encode_seed()).unwrap();

        assert_eq!(decoded.server_id(), identity.server_id());
        assert_eq!(identity.server_id().len(), 16);
    }

    #[test]
    fn a_missing_seed_is_created_and_reloaded_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pairing-secret");

        let created = ServerIdentity::load_or_create_at(&path).unwrap();
        let reloaded = ServerIdentity::load_or_create_at(&path).unwrap();

        assert_eq!(created.server_id(), reloaded.server_id());
        assert!(!path.with_extension("unreadable").exists());
    }

    #[test]
    fn an_undecodable_seed_is_set_aside_and_replaced() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pairing-secret");
        fs::write(&path, "CORRUPTED-NOT-BASE64").unwrap();

        let replacement = ServerIdentity::load_or_create_at(&path).unwrap();

        assert_eq!(
            fs::read_to_string(path.with_extension("unreadable")).unwrap(),
            "CORRUPTED-NOT-BASE64"
        );
        assert_eq!(
            ServerIdentity::load_or_create_at(&path)
                .unwrap()
                .server_id(),
            replacement.server_id()
        );
    }

    #[test]
    fn a_non_utf8_seed_is_set_aside_and_replaced() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pairing-secret");
        fs::write(&path, [0xff, 0xfe, 0xfd]).unwrap();

        ServerIdentity::load_or_create_at(&path).unwrap();

        assert_eq!(
            fs::read(path.with_extension("unreadable")).unwrap(),
            [0xff, 0xfe, 0xfd]
        );
    }

    #[test]
    fn a_seed_of_the_wrong_length_is_set_aside_and_replaced() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pairing-secret");
        let too_short = URL_SAFE_NO_PAD.encode([1; 16]);
        fs::write(&path, &too_short).unwrap();

        ServerIdentity::load_or_create_at(&path).unwrap();

        assert_eq!(
            fs::read_to_string(path.with_extension("unreadable")).unwrap(),
            too_short
        );
        assert_ne!(fs::read_to_string(&path).unwrap(), too_short);
    }

    #[test]
    fn a_seed_path_that_is_not_a_regular_file_is_never_replaced() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pairing-secret");
        fs::create_dir(&path).unwrap();

        assert!(ServerIdentity::load_or_create_at(&path).is_err());
        assert!(path.is_dir());
        assert!(!path.with_extension("unreadable").exists());
    }

    #[test]
    fn inspecting_a_missing_seed_never_creates_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("missing-secret");

        let inspection = inspect_path(&path);

        assert_eq!(inspection.state, ExistingSecretState::Missing);
        assert!(!path.exists());
    }

    #[test]
    fn inspecting_a_regular_seed_reads_metadata_but_not_contents() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pairing-secret");
        let private_non_utf8 = [0xff, 0xfe, 0xfd, 0x00];
        fs::write(&path, private_non_utf8).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();

        let inspection = inspect_path(&path);

        assert_eq!(inspection.state, ExistingSecretState::Present);
        assert_eq!(inspection.bytes, Some(private_non_utf8.len() as u64));
        assert_eq!(fs::read(&path).unwrap(), private_non_utf8);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
    }
}
