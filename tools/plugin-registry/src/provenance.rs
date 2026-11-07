use anyhow::{Context, Result};
use qol_plugin_index::Blob;
use std::collections::BTreeMap;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

pub const MAIN_REF: &str = "refs/heads/main";

static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);

pub trait Provenance: Sync {
    fn verify(&self, bytes: &[u8], source_ref: &str) -> Result<()>;
}

pub struct Provenances<'a> {
    pub plugin: &'a dyn Provenance,
    pub host: &'a dyn Provenance,
}

pub struct GhAttestation {
    pub repo: String,
    pub workflow: String,
}

impl Provenance for GhAttestation {
    fn verify(&self, bytes: &[u8], source_ref: &str) -> Result<()> {
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
            .args(["--source-ref", source_ref, "--deny-self-hosted-runners"])
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
    assets: &BTreeMap<String, Blob>,
    download: impl Fn(&Blob) -> Result<Vec<u8>>,
    provenance: &dyn Provenance,
    source_refs: &[&str],
) -> Result<()> {
    for (name, blob) in assets {
        let bytes = download(blob)?;
        let mut refusal = anyhow::anyhow!("no source ref was given");
        let verified =
            source_refs
                .iter()
                .any(|source_ref| match provenance.verify(&bytes, source_ref) {
                    Ok(()) => true,
                    Err(error) => {
                        refusal = error;
                        false
                    }
                });
        if !verified {
            return Err(refusal).with_context(|| format!("{name} has no valid build provenance"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Refuses {
        bytes: &'static [u8],
        accepted_ref: Option<&'static str>,
        seen: Mutex<Vec<String>>,
    }

    impl Refuses {
        fn new(bytes: &'static [u8], accepted_ref: Option<&'static str>) -> Self {
            Self {
                bytes,
                accepted_ref,
                seen: Mutex::new(Vec::new()),
            }
        }
    }

    impl Provenance for Refuses {
        fn verify(&self, bytes: &[u8], source_ref: &str) -> Result<()> {
            self.seen.lock().unwrap().push(source_ref.to_string());
            if bytes == self.bytes {
                anyhow::bail!("not attested");
            }
            match self.accepted_ref {
                Some(accepted) if accepted != source_ref => {
                    anyhow::bail!("built from another ref")
                }
                _ => Ok(()),
            }
        }
    }

    fn assets(assets: &[(&str, &str)]) -> BTreeMap<String, Blob> {
        assets
            .iter()
            .map(|(name, digest)| {
                let blob = Blob {
                    digest: digest.to_string(),
                    size: 1,
                };
                (name.to_string(), blob)
            })
            .collect()
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
        for (case, listed, failure) in cases {
            let provenance = Refuses::new(b"b", None);
            let result = verify_assets(&assets(listed), download, &provenance, &[MAIN_REF]);
            match failure {
                None => assert!(result.is_ok(), "{case}: {result:?}"),
                Some(failure) => {
                    let error = format!("{:#}", result.unwrap_err());
                    assert!(error.contains(failure), "{case}: {error}");
                }
            }
        }
    }

    #[test]
    fn an_asset_passes_when_any_listed_ref_verifies() {
        let download = |_: &Blob| Ok(b"a".to_vec());
        let listed = assets(&[("tray", "sha256:a")]);
        let tag_ref = "refs/tags/qol-shot-v1.0.0";
        let cases: &[(&str, &str, &[&str], &[&str], Option<&str>)] = &[
            ("first ref", tag_ref, &[tag_ref, MAIN_REF], &[tag_ref], None),
            (
                "second ref",
                MAIN_REF,
                &[tag_ref, MAIN_REF],
                &[tag_ref, MAIN_REF],
                None,
            ),
            (
                "no ref",
                "refs/heads/other",
                &[tag_ref, MAIN_REF],
                &[tag_ref, MAIN_REF],
                Some("built from another ref"),
            ),
            (
                "no refs given",
                MAIN_REF,
                &[],
                &[],
                Some("no source ref was given"),
            ),
        ];
        for (case, accepted_ref, source_refs, tried, failure) in cases {
            let provenance = Refuses::new(b"b", Some(*accepted_ref));
            let result = verify_assets(&listed, download, &provenance, source_refs);
            match failure {
                None => assert!(result.is_ok(), "{case}: {result:?}"),
                Some(failure) => {
                    let error = format!("{:#}", result.unwrap_err());
                    assert!(
                        error.contains("tray has no valid build provenance")
                            && error.contains(failure),
                        "{case}: {error}"
                    );
                }
            }
            assert_eq!(*provenance.seen.lock().unwrap(), *tried, "{case}");
        }
    }

    #[test]
    fn each_asset_is_downloaded_once() {
        let downloads = Mutex::new(0);
        let download = |_: &Blob| {
            *downloads.lock().unwrap() += 1;
            Ok(b"a".to_vec())
        };
        let provenance = Refuses::new(b"b", Some(MAIN_REF));
        let listed = assets(&[("tray", "sha256:a"), ("installer", "sha256:b")]);
        verify_assets(&listed, download, &provenance, &["refs/tags/x", MAIN_REF]).unwrap();
        assert_eq!(*downloads.lock().unwrap(), 2);
    }
}
