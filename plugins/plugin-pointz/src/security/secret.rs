use anyhow::{Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

const SECRET_FILE: &str = "pairing-secret";

pub struct PairingSecret([u8; 32]);

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
}

fn secret_path() -> Option<std::path::PathBuf> {
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

    #[test]
    fn encoded_secret_round_trips_and_has_stable_server_id() {
        let secret = PairingSecret::from_bytes([3; 32]);

        let decoded = PairingSecret::decode(&secret.encoded()).unwrap();

        assert_eq!(decoded.bytes(), secret.bytes());
        assert_eq!(decoded.server_id(), secret.server_id());
        assert_eq!(secret.server_id().len(), 16);
    }
}
