use anyhow::{Context, Result};
use qol_plugin_index::{Blob, IndexedHostVersion};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

// Versioning only releases from main, so every genuine attestation names it.
const RELEASE_REF: &str = "refs/heads/main";

static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);

pub trait Provenance: Sync {
    fn verify(&self, bytes: &[u8]) -> Result<()>;
}

pub struct GhAttestation {
    pub repo: String,
    pub workflow: String,
}

impl Provenance for GhAttestation {
    fn verify(&self, bytes: &[u8]) -> Result<()> {
        let file = std::env::temp_dir().join(format!(
            "qol-provenance-{}-{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&file, bytes).with_context(|| format!("cannot write {}", file.display()))?;
        let output = Command::new("gh")
            .arg("attestation")
            .arg("verify")
            .arg(&file)
            .args(["--repo", &self.repo])
            .args([
                "--signer-workflow",
                &format!("{}/{}", self.repo, self.workflow),
            ])
            .args(["--source-ref", RELEASE_REF, "--deny-self-hosted-runners"])
            .output();
        std::fs::remove_file(&file).ok();
        let output = output.context("could not run gh attestation verify")?;
        if !output.status.success() {
            anyhow::bail!(
                "gh attestation verify refused it: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }
}

pub fn verify_assets(
    entry: &IndexedHostVersion,
    download: impl Fn(&Blob) -> Result<Vec<u8>>,
    provenance: &dyn Provenance,
) -> Result<()> {
    for (name, blob) in &entry.assets {
        let bytes = download(blob)?;
        provenance
            .verify(&bytes)
            .with_context(|| format!("{name} has no valid build provenance"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct Refuses(&'static [u8]);

    impl Provenance for Refuses {
        fn verify(&self, bytes: &[u8]) -> Result<()> {
            if bytes == self.0 {
                anyhow::bail!("not attested");
            }
            Ok(())
        }
    }

    fn entry(assets: &[(&str, &str)]) -> IndexedHostVersion {
        IndexedHostVersion {
            manifest_digest: "sha256:m".to_string(),
            revision: "r".to_string(),
            assets: assets
                .iter()
                .map(|(name, digest)| {
                    let blob = Blob {
                        digest: digest.to_string(),
                        size: 1,
                    };
                    (name.to_string(), blob)
                })
                .collect(),
        }
    }

    type Case = (
        &'static str,
        &'static [(&'static str, &'static str)],
        Option<&'static str>,
    );

    #[test]
    fn every_asset_must_carry_provenance() {
        let blobs = BTreeMap::from([("sha256:a", b"a".to_vec()), ("sha256:b", b"b".to_vec())]);
        let download = |blob: &Blob| {
            blobs
                .get(blob.digest.as_str())
                .cloned()
                .context("missing blob")
        };
        let cases: &[Case] = &[
            ("all attested", &[("tray", "sha256:a")], None),
            (
                "one unattested",
                &[("tray", "sha256:a"), ("installer", "sha256:b")],
                Some("installer has no valid build provenance"),
            ),
            (
                "download fails",
                &[("tray", "sha256:c")],
                Some("missing blob"),
            ),
        ];
        for (case, assets, failure) in cases {
            let result = verify_assets(&entry(assets), download, &Refuses(b"b"));
            match failure {
                None => assert!(result.is_ok(), "{case}: {result:?}"),
                Some(failure) => {
                    let error = format!("{:#}", result.unwrap_err());
                    assert!(error.contains(failure), "{case}: {error}");
                }
            }
        }
    }
}
