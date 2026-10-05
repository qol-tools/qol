use super::document::{self, IndexDocument};
use super::IndexLocation;
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SERIALS_FILE: &str = "plugin-index-serials.json";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) async fn load(location: &IndexLocation) -> Result<IndexDocument> {
    let client = http_client();
    let body = get(&client, &location.url).await?.bytes().await?;
    let signature_url = format!("{}.minisig", location.url);
    let signature = get(&client, &signature_url).await?.text().await?;
    let document = document::verify(&body, &signature, &location.public_key)?;
    accept_serial(&serials_path()?, &location.url, document.serial)?;
    qol_runtime::probe!(
        "PLUGIN_INDEX",
        "event=loaded url={} serial={} plugins={}",
        location.url,
        document.serial,
        document.plugins.len()
    );
    Ok(document)
}

pub(super) fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .user_agent("qol-tray")
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

pub(super) async fn get(client: &reqwest::Client, url: &str) -> Result<reqwest::Response> {
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("could not reach {url}"))?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("{url} answered {status}");
    }
    Ok(response)
}

fn serials_path() -> Result<PathBuf> {
    Ok(crate::paths::base_data_dir()?.join(SERIALS_FILE))
}

fn accept_serial(path: &Path, url: &str, serial: u64) -> Result<()> {
    let mut serials = read_serials(path);
    let seen = serials.get(url).copied().unwrap_or(0);
    if serial < seen {
        anyhow::bail!(
            "the plugin index at {url} is older than one already accepted (serial {serial} < {seen})"
        );
    }
    if serial == seen {
        return Ok(());
    }
    serials.insert(url.to_string(), serial);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    qol_fs::atomic_write(path, &serde_json::to_vec(&serials)?).with_context(|| {
        format!(
            "failed to record the plugin index serial in {}",
            path.display()
        )
    })
}

fn read_serials(path: &Path) -> BTreeMap<String, u64> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accept_serial_refuses_only_an_older_index_per_url() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("state").join(SERIALS_FILE);
        let steps: &[(&str, u64, bool)] = &[
            ("https://a/index.json", 10, true),
            ("https://a/index.json", 10, true),
            ("https://a/index.json", 12, true),
            ("https://a/index.json", 11, false),
            ("https://b/index.json", 1, true),
            ("https://a/index.json", 12, true),
        ];
        for (url, serial, accepted) in steps {
            assert_eq!(
                accept_serial(&path, url, *serial).is_ok(),
                *accepted,
                "url={url} serial={serial}"
            );
        }
        let stored = read_serials(&path);
        assert_eq!(stored.get("https://a/index.json"), Some(&12));
        assert_eq!(stored.get("https://b/index.json"), Some(&1));
    }
}
