use anyhow::{Context, Result};
use qol_plugin_index::Blob;
use std::collections::BTreeMap;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

pub const MAIN_REF: &str = "refs/heads/main";

static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);

pub struct Source<'a> {
    pub reference: &'a str,
    pub digest: Option<&'a str>,
}

pub trait Provenance: Sync {
    fn verify(&self, bytes: &[u8], source: &Source<'_>) -> Result<()>;
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
    fn verify(&self, bytes: &[u8], source: &Source<'_>) -> Result<()> {
        let file = std::env::temp_dir().join(format!(
            "qol-provenance-{}-{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&file, bytes).with_context(|| format!("cannot write {}", file.display()))?;
        let mut command = Command::new("gh");
        command
            .arg("attestation")
            .arg("verify")
            .arg(&file)
            .args(["--repo", &self.repo])
            .args([
                "--signer-workflow",
                &format!("{}/{}", self.repo, self.workflow),
            ])
            .args([
                "--source-ref",
                source.reference,
                "--deny-self-hosted-runners",
            ]);
        if let Some(digest) = source.digest {
            command.args(["--source-digest", digest]);
        }
        let output = command.output();
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
    source: &Source<'_>,
) -> Result<()> {
    for (name, blob) in assets {
        let bytes = download(blob)?;
        provenance
            .verify(&bytes, source)
            .with_context(|| format!("{name} has no valid build provenance"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: Source<'static> = Source {
        reference: MAIN_REF,
        digest: None,
    };
    const TAG_REF: &str = "refs/tags/qol-shot-v1.0.0";
    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    struct Refuses {
        bytes: &'static [u8],
        built_from: Option<(&'static str, &'static str)>,
    }

    impl Provenance for Refuses {
        fn verify(&self, bytes: &[u8], source: &Source<'_>) -> Result<()> {
            if bytes == self.bytes {
                anyhow::bail!("not attested");
            }
            match self.built_from {
                Some((reference, _)) if reference != source.reference => {
                    anyhow::bail!("built from another ref")
                }
                Some((_, digest)) if source.digest.is_some_and(|wanted| wanted != digest) => {
                    anyhow::bail!("built from another commit")
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
            let provenance = Refuses {
                bytes: b"b",
                built_from: None,
            };
            let result = verify_assets(&assets(listed), download, &provenance, &MAIN);
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
    fn an_asset_must_come_from_the_named_ref_and_commit() {
        let download = |_: &Blob| Ok(b"a".to_vec());
        let listed = assets(&[("tray", "sha256:a")]);
        let other = "fedcba9876543210fedcba9876543210fedcba98";
        let cases: &[(&str, Source<'_>, Option<&str>)] = &[
            (
                "tag and commit",
                Source {
                    reference: TAG_REF,
                    digest: Some(COMMIT),
                },
                None,
            ),
            (
                "tag only",
                Source {
                    reference: TAG_REF,
                    digest: None,
                },
                None,
            ),
            (
                "another ref",
                Source {
                    reference: MAIN_REF,
                    digest: Some(COMMIT),
                },
                Some("built from another ref"),
            ),
            (
                "another commit",
                Source {
                    reference: TAG_REF,
                    digest: Some(other),
                },
                Some("built from another commit"),
            ),
        ];
        for (case, source, failure) in cases {
            let provenance = Refuses {
                bytes: b"b",
                built_from: Some((TAG_REF, COMMIT)),
            };
            let result = verify_assets(&listed, download, &provenance, source);
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
        }
    }
}
