use super::document::{self, IndexDocument};
use super::IndexLocation;
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

const SERIALS_FILE: &str = "plugin-index-serials.json";
const KEPT_DIR: &str = "plugin-index-kept";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const READ_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) async fn load(location: &IndexLocation) -> Result<IndexDocument> {
    let client = http_client();
    let signed = read_body(&location.url, get(&client, &location.url).await?.bytes()).await?;
    let served = document::verify_signed(&signed, &location.public_key)?;
    let served_serial = served.serial;
    let document = keep_newest(&crate::paths::base_data_dir()?, location, served, &signed)?;
    qol_runtime::probe!(
        "PLUGIN_INDEX",
        "event=loaded url={} serial={} served={} plugins={}",
        location.url,
        document.serial,
        served_serial,
        document.plugins.len()
    );
    Ok(document)
}

pub(super) fn http_client() -> reqwest::Client {
    build_client(reqwest::Client::builder().timeout(REQUEST_TIMEOUT))
}

pub(super) fn blob_client() -> reqwest::Client {
    build_client(
        reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT),
    )
}

fn build_client(builder: reqwest::ClientBuilder) -> reqwest::Client {
    builder
        .user_agent("qol-tray")
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

pub(super) async fn get(client: &reqwest::Client, url: &str) -> Result<reqwest::Response> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| anyhow::anyhow!("could not reach {url}: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow::anyhow!("{url} answered {status}"));
    }
    Ok(response)
}

pub(super) async fn read_body<T>(
    url: &str,
    body: impl std::future::Future<Output = reqwest::Result<T>>,
) -> Result<T> {
    body.await
        .map_err(|error| anyhow::anyhow!("could not read {url}: {error}"))
}

fn keep_newest(
    data_dir: &Path,
    location: &IndexLocation,
    served: IndexDocument,
    signed: &[u8],
) -> Result<IndexDocument> {
    let serials_path = data_dir.join(SERIALS_FILE);
    let mut serials = read_serials(&serials_path);
    let seen = serials.get(&location.url).copied().unwrap_or(0);
    let kept_path = data_dir.join(KEPT_DIR).join(kept_name(&location.url));
    if served.serial < seen {
        return kept(&kept_path, location, served.serial, seen);
    }
    if served.serial > seen || !kept_path.exists() {
        write(&kept_path, signed, "the plugin index")?;
    }
    if served.serial > seen {
        serials.insert(location.url.clone(), served.serial);
        write(
            &serials_path,
            &serde_json::to_vec(&serials)?,
            "the plugin index serial",
        )?;
    }
    Ok(served)
}

fn kept(path: &Path, location: &IndexLocation, served: u64, seen: u64) -> Result<IndexDocument> {
    let url = &location.url;
    let older = || {
        anyhow::anyhow!(
            "the plugin index at {url} is older than one already accepted (serial {served} < {seen})"
        )
    };
    let signed = std::fs::read(path).map_err(|_| older())?;
    let document = document::verify_signed(&signed, &location.public_key).with_context(|| {
        format!(
            "the plugin index kept in {} does not verify",
            path.display()
        )
    })?;
    if document.serial != seen {
        return Err(older());
    }
    Ok(document)
}

fn kept_name(url: &str) -> String {
    format!("{:x}.json", Sha256::digest(url.as_bytes()))
}

fn write(path: &Path, bytes: &[u8], what: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    qol_fs::atomic_write(path, bytes)
        .with_context(|| format!("failed to record {what} in {}", path.display()))
}

fn read_serials(path: &Path) -> BTreeMap<String, u64> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::super::document::fixtures::{document, Signer};
    use super::*;

    fn location(url: &str, signer: &Signer) -> IndexLocation {
        IndexLocation {
            url: url.to_string(),
            public_key: signer.public_key(),
        }
    }

    fn signed(signer: &Signer, serial: u64) -> Vec<u8> {
        let body = document(serial, "https://ghcr.io", serde_json::json!({}));
        signer.sign_index(&serde_json::to_vec(&body).unwrap())
    }

    fn load(
        data_dir: &Path,
        location: &IndexLocation,
        signer: &Signer,
        serial: u64,
    ) -> Result<u64> {
        let bytes = signed(signer, serial);
        let served = document::verify_signed(&bytes, &location.public_key)?;
        keep_newest(data_dir, location, served, &bytes).map(|document| document.serial)
    }

    #[test]
    fn an_older_index_is_answered_with_the_newest_one_accepted_per_url() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("state");
        let signer = Signer::generate();
        let a = location("https://a/index.signed.json", &signer);
        let b = location("https://b/index.signed.json", &signer);
        let steps: &[(&IndexLocation, u64, u64)] = &[
            (&a, 10, 10),
            (&a, 10, 10),
            (&a, 12, 12),
            (&a, 11, 12),
            (&b, 1, 1),
            (&a, 12, 12),
            (&a, 11, 12),
        ];
        for (location, served, expected) in steps {
            assert_eq!(
                load(&data_dir, location, &signer, *served).unwrap(),
                *expected,
                "url={} served={served}",
                location.url
            );
        }
        let stored = read_serials(&data_dir.join(SERIALS_FILE));
        assert_eq!(stored.get(&a.url), Some(&12));
        assert_eq!(stored.get(&b.url), Some(&1));
    }

    #[test]
    fn an_older_index_fails_when_the_kept_one_is_gone_or_not_signed() {
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("state");
        let signer = Signer::generate();
        let a = location("https://a/index.signed.json", &signer);
        let kept_path = data_dir.join(KEPT_DIR).join(kept_name(&a.url));
        load(&data_dir, &a, &signer, 12).unwrap();

        std::fs::write(&kept_path, signed(&Signer::generate(), 12)).unwrap();
        let forged = load(&data_dir, &a, &signer, 11).unwrap_err();
        std::fs::write(&kept_path, signed(&signer, 10)).unwrap();
        let stale = load(&data_dir, &a, &signer, 11).unwrap_err();
        std::fs::remove_file(&kept_path).unwrap();
        let gone = load(&data_dir, &a, &signer, 11).unwrap_err();

        for (error, cause) in [
            (forged, "does not verify"),
            (stale, "older than one already accepted (serial 11 < 12)"),
            (gone, "older than one already accepted (serial 11 < 12)"),
        ] {
            assert!(format!("{error:#}").contains(cause), "{error:#}");
        }
        assert_eq!(load(&data_dir, &a, &signer, 13).unwrap(), 13);
        assert_eq!(load(&data_dir, &a, &signer, 12).unwrap(), 13);
    }
}
