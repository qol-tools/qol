use crate::registry::{read_body, send};
use anyhow::{Context, Result};
use qol_plugin_index::IndexDocument;

pub fn fetch(agent: &ureq::Agent, url: &str, public_key: &str) -> Result<Option<IndexDocument>> {
    let response = send(agent.get(url), None, url)?;
    match response.status() {
        404 => return Ok(None),
        200 => {}
        status => anyhow::bail!("{url} answered {status}"),
    }
    let body = read_body(response, url)?;
    let signature_url = format!("{url}.minisig");
    let response = send(agent.get(&signature_url), None, &signature_url)?;
    if response.status() != 200 {
        anyhow::bail!("{signature_url} answered {}", response.status());
    }
    let signature = String::from_utf8(read_body(response, &signature_url)?)
        .with_context(|| format!("{signature_url} is not text"))?;
    let document = qol_plugin_index::verify(&body, &signature, public_key)
        .with_context(|| format!("the deployed index at {url} does not verify"))?;
    Ok(Some(document))
}
