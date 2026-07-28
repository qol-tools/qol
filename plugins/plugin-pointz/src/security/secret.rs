use anyhow::{Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

const SECRET_FILE: &str = "pairing-secret";

pub struct PairingSecret([u8; 32]);

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

impl PairingSecret {
    pub fn load_or_create() -> Result<Self> {
        let path = secret_path().context("PointZ data directory is unavailable")?;
        match std::fs::symlink_metadata(&path) {
            Ok(metadata)
                if metadata.file_type().is_file() && !metadata.file_type().is_symlink() =>
            {
                return Self::decode(std::fs::read_to_string(path)?.trim());
            }
            Ok(_) => anyhow::bail!("PointZ pairing secret path is not a regular file"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|error| {
            anyhow::anyhow!("failed to generate PointZ pairing secret: {error}")
        })?;
        let secret = Self(bytes);
        qol_fs::atomic_write_private(&path, secret.encoded().as_bytes()).with_context(|| {
            format!(
                "failed to persist PointZ pairing secret at {}",
                path.display()
            )
        })?;
        Ok(secret)
    }

    pub fn bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn encoded(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }

    pub fn server_id(&self) -> String {
        let digest = Sha256::digest(self.0);
        URL_SAFE_NO_PAD.encode(&digest[..12])
    }

    fn decode(encoded: &str) -> Result<Self> {
        let decoded = URL_SAFE_NO_PAD
            .decode(encoded)
            .context("PointZ pairing secret is not valid base64url")?;
        let bytes: [u8; 32] = decoded
            .try_into()
            .map_err(|_| anyhow::anyhow!("PointZ pairing secret must be 32 bytes"))?;
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
        issue: Some("PointZ pairing secret path is not a regular file".to_string()),
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
    fn encoded_secret_round_trips_and_has_stable_server_id() {
        let secret = PairingSecret::from_bytes([3; 32]);

        let decoded = PairingSecret::decode(&secret.encoded()).unwrap();

        assert_eq!(decoded.bytes(), secret.bytes());
        assert_eq!(decoded.server_id(), secret.server_id());
        assert_eq!(secret.server_id().len(), 16);
    }

    #[test]
    fn inspecting_a_missing_secret_never_creates_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("missing-secret");

        let inspection = inspect_path(&path);

        assert_eq!(inspection.state, ExistingSecretState::Missing);
        assert!(!path.exists());
    }

    #[test]
    fn inspecting_a_regular_secret_reads_metadata_but_not_contents() {
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

    #[test]
    fn inspecting_a_regular_secret_never_exposes_private_material() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("pairing-secret");
        let private_material = "private-but-uninspected-material";
        fs::write(&path, private_material).unwrap();

        let inspection = inspect_path(&path);

        assert_eq!(inspection.state, ExistingSecretState::Present);
        assert!(!format!("{inspection:?}").contains(private_material));
        assert_eq!(fs::read_to_string(path).unwrap(), private_material);
    }
}
